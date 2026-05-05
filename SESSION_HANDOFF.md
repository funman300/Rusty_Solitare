# Solitaire Quest — Session Handoff

**Last updated:** 2026-05-02 (post-v0.15.0) — In-engine replay playback, the Klondike solver + Winnable-deals toggle, a 19th achievement (Cinephile), rolling replay history, and the Bevy default-features trim all shipped under v0.15.0. Direction now opens.

## Status at pause

- **HEAD on origin:** v0.15.0's tag commit.
- **Working tree:** clean apart from untracked `CARD_PLAN.md` (intentional).
- **Build:** `cargo clippy --workspace --all-targets -- -D warnings` clean.
- **Tests:** **1178 passed / 0 failed** across the workspace.
- **Tags on origin:** `v0.9.0`, `v0.10.0`, `v0.11.0`, `v0.12.0`, `v0.13.0`, `v0.14.0`, `v0.15.0`.

## Where we are

v0.15.0 closes out the post-v0.14.0 candidate list: every item the prior handoff seeded shipped. The major new player-facing pieces are the working "Watch replay" button (in-engine playback with a Stop button overlay), a hand-rolled Klondike solver with the optional "Winnable deals only" toggle, and a rolling history of the last 8 wins. The under-the-hood win is the Bevy default-features trim that drops 51 transitive crates.

The post-v0.15.0 candidate list is short — solver-driven hints (now possible since the solver exists), desktop packaging (still pending artwork + signing certs), and a fresh round of UX iteration. Direction is open.

### Design direction (unchanged)

- **Tone:** Balatro — chunky readable type, theatrical hierarchy, satisfying micro-interactions.
- **Palette:** Midnight Purple base + Balatro yellow primary + warm magenta secondary.
- See `~/.claude/projects/-home-manage-Rusty-Solitare/memory/project_ux_overhaul_2026-04.md` (machine-local).

### Canonical remote

`github.com/funman300/Rusty_Solitaire` is the canonical repo. Always push there.

## v0.15.0 (shipped 2026-05-02)

| Area | Commit | What landed |
|---|---|---|
| Bevy trim | `95fcdad` | `default-features = false` plus a curated explicit feature list. Drops 51 transitive crates including the `bevy_audio` → rodio → cpal 0.15 + symphonia chain (kira handles audio directly). `solitaire_wasm` is bevy-free and unaffected. |
| Replay playback core | `8e90574` | `ReplayPlaybackPlugin` + `ReplayPlaybackState` enum. Iterative DFS through `replay.moves` at `REPLAY_MOVE_INTERVAL_SECS` (0.45 s) firing canonical events. Recording suppression via length-truncation in a sibling system — `game_plugin` untouched. Reset-to-recorded-deal uses direct `GameStateResource` insert to apply the recording's exact `draw_mode`. |
| Replay overlay UI | `9c36b49` | Top-anchored banner (`ReplayOverlayPlugin`): "Replay" label + "Move N of M" progress + Tertiary Stop button. Z = 55 (above HUD, below modals so Settings / Pause / Help still open during playback). |
| Stats button wiring | `02ababa` | Watch Replay button now calls `start_replay_playback` instead of firing a stub toast. `Option<ResMut<ReplayPlaybackState>>` so headless tests without `ReplayPlaybackPlugin` still pass. |
| Replay history | `13a8a01` | Rolling list of 8 wins at `<data_dir>/replays.json`. Legacy `latest_replay.json` migrates forward on first launch via `migrate_legacy_latest_replay`. Stats overlay's selector — Prev / Next chips + "Replay N / M" caption — lets the player step through older wins. |
| Cinephile achievement | `bf660df` | 19th achievement; unlocks on `Playing → Completed` transition (not on Stop, which goes Playing → Inactive). README count + ARCHITECTURE.md §11 entry updated. |
| Solver + toggle | `8a5fa87` | `solitaire_core::solver::try_solve(seed, draw_mode, &SolverConfig) -> SolverResult { Winnable, Unwinnable, Inconclusive }`. Iterative DFS, 64-bit canonical state hash, priority-ordered move enumeration, two budget knobs (100k moves / 200k states default). Median solve 2 ms, pathological 120 ms. Settings → Gameplay toggle "Winnable deals only" (default off) makes `handle_new_game` retry seeds up to `SOLVER_DEAL_RETRY_CAP = 50` attempts. Daily / replays / explicit-seed bypass the solver. |

## Open punch list

### Release prep

1. **Smoke-test on a real game**: confirm the new replay playback feels right at 0.45 s/move; verify the Winnable-deals toggle doesn't introduce a visible stall on a typical machine; try the rolling-history selector.
2. **Desktop packaging** per `ARCHITECTURE.md §17`. Arch PKGBUILD exists in `/home/manage/solitaire-quest-pkgbuild/` (separate repo). Pending: app icon, macOS `.icns` + notarisation cert, Windows `.ico` + Authenticode cert, AppImage recipe.

### Next-round candidates

- **Solver-driven hints** — the existing hint system uses a heuristic; promote it to ask `try_solve` for the actual best move. Scope: small wrapper around the solver's `enumerate_moves` plus the existing hint plumbing. Now unblocked.
- **Replay-playback rate slider** — the 0.45 s/move pace is hardcoded; a Settings slider in the same row as tooltip-delay / time-bonus would let power users speed up older replays.
- **Solver progress overlay** — when "Winnable deals only" is on, a brief "checking deal…" toast surfaces after ~500 ms so the player isn't confused by the rare worst-case stall.
- **Solver-on-AsyncComputeTaskPool** — current solver runs synchronously on the main thread. Worst-case 50 attempts × 120 ms = 6 s of UI stall on pathological seeds. Async + cancel button would be safer.
- **Per-deal "won previously" indicator** — the rolling replay history's seeds make this easy: when a new game starts on a seed the player has already won, surface a tiny indicator on the HUD.
- **Replay sharing** — `replays.json` is per-machine. Allow a player to copy a replay's URL (already wired via `solitaire_server`) and post it elsewhere. The web-viewer already exists.

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
Branch: master. Direction is OPEN — v0.15.0 just shipped covering
in-engine replay playback, the Klondike solver + Winnable-deals
toggle, replay history, the Cinephile achievement, and the Bevy
default-features trim.

State: HEAD at v0.15.0. Working tree clean apart from untracked
CARD_PLAN.md (intentional).
Build: cargo clippy --workspace --all-targets -- -D warnings clean.
Tests: 1178 passed / 0 failed.

READ FIRST (in order, before doing anything):
  1. SESSION_HANDOFF.md  — v0.15.0 changelog + open punch list
  2. CHANGELOG.md        — release-by-release record
  3. CLAUDE.md           — hard rules (UI-first, no panics, etc.)
  4. ARCHITECTURE.md     — crate responsibilities + data flow
  5. ~/.claude/projects/<this-project>/memory/MEMORY.md
                         — saved feedback / project context (machine-local;
                           may be missing on a fresh machine)

DECISION TO ASK THE PLAYER FIRST:
  A. Smoke-test v0.15.0 in a real game session. Solver, replay
     playback, replay history selector, Cinephile achievement.
  B. Take solver-driven hints (now possible with the solver in place).
     Replace the heuristic hint with `try_solve`'s best-move
     suggestion.
  C. Move the solver to AsyncComputeTaskPool with a "checking deal…"
     progress toast and a cancel button. Eliminates the worst-case
     6 s UI stall.
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
