# Solitaire Quest — Session Handoff

**Last updated:** 2026-05-06 (post-v0.16.0) — Modal-feel polish round shipped: every overlay scrolls when it overflows, every button shows a pointer cursor on hover, modal focus lands on the same frame, and read-only modals dismiss on scrim click. Direction now opens.

## Status at pause

- **HEAD on origin:** v0.16.0's tag commit.
- **Working tree:** clean apart from untracked `CARD_PLAN.md` (intentional).
- **Build:** `cargo clippy --workspace --all-targets -- -D warnings` clean.
- **Tests:** **1196 passed / 0 failed** across the workspace.
- **Tags on origin:** `v0.9.0` through `v0.16.0`.

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

## Open punch list

### Release prep

1. **Smoke-test on a real game**: confirm scroll feels right on Achievements (the original bug), pointer cursor changes on every interactive surface, the very first Tab in a modal already activates the primary, and clicking the dimmed area dismisses the read-only modals while NOT dismissing Settings/Pause.
2. **Desktop packaging** per `ARCHITECTURE.md §17`. Arch PKGBUILD exists in `/home/manage/solitaire-quest-pkgbuild/` (separate repo). Pending: app icon, macOS `.icns` + notarisation cert, Windows `.ico` + Authenticode cert, AppImage recipe.

### Carryover from v0.15.0 next-round candidates

Still open — would each be ~50–200 LOC:

- **Solver-driven hints** — the existing hint system uses a heuristic; promote it to ask `try_solve` for the actual best move. Now that the solver is in place this is mostly plumbing.
- **Replay-playback rate slider** — the 0.45 s/move pace is hardcoded; a Settings slider in the same row as tooltip-delay / time-bonus would let power users speed up older replays.
- **Solver progress overlay** — when "Winnable deals only" is on, a brief "checking deal…" toast surfaces after ~500 ms so the player isn't confused by the rare worst-case stall.
- **Solver-on-AsyncComputeTaskPool** — current solver runs synchronously on the main thread. Worst-case 50 attempts × 120 ms = 6 s of UI stall on pathological seeds. Async + cancel button would be safer.
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

State: HEAD at v0.16.0. Working tree clean apart from untracked
CARD_PLAN.md (intentional).
Build: cargo clippy --workspace --all-targets -- -D warnings clean.
Tests: 1196 passed / 0 failed.

READ FIRST (in order, before doing anything):
  1. SESSION_HANDOFF.md  — v0.16.0 changelog + open punch list
  2. CHANGELOG.md        — release-by-release record
  3. CLAUDE.md           — hard rules (UI-first, no panics, etc.)
  4. ARCHITECTURE.md     — crate responsibilities + data flow
  5. ~/.claude/projects/<this-project>/memory/MEMORY.md
                         — saved feedback / project context (machine-local;
                           may be missing on a fresh machine)

DECISION TO ASK THE PLAYER FIRST:
  A. Smoke-test v0.16.0. Scroll on Achievements, pointer cursor on
     buttons, first Tab in a modal activates rather than advances,
     scrim click dismisses Stats/Achievements/Help/Profile/
     Leaderboard/Home but NOT Settings/Pause/etc.
  B. Solver-driven hints — replace heuristic with try_solve's
     best-move suggestion. ~100 LOC.
  C. Solver-on-AsyncComputeTaskPool with progress toast + cancel.
     Eliminates the worst-case 6 s stall.
  D. Pick from the remaining "next-round candidates" in this doc.
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
