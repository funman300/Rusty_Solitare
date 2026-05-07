# Solitaire Quest — Session Handoff

**Last updated:** 2026-05-06 (post-v0.18.0, [Unreleased] accumulating
v0.19.0 candidates) — v0.18.0 tagged + pushed at `bfcd05f`. Three
commits sit on top: the H-key hint moved onto
`AsyncComputeTaskPool` (closing the last synchronous solver hot
path), persistent replay share URLs (no more
in-session-only sharing), and a fix for the
`auto_save_writes_after_30_seconds` test flake.

## Status at pause

- **HEAD on origin:** `42d90b1` (the persistent share-link
  commit). Local HEAD is one ahead at `91b7605` (auto-save flake
  fix), with this round's `[Unreleased]` doc refresh staged on
  top.
- **Working tree:** modified — `CHANGELOG.md` and
  `SESSION_HANDOFF.md` carry the `[Unreleased]` doc updates.
- **Build:** `cargo clippy --workspace --all-targets -- -D warnings`
  clean (verified this session).
- **Tests:** **1170 passing / 0 failing** across the workspace
  (verified this session).
  `auto_save_writes_after_30_seconds` reverified stable across
  three back-to-back runs after the flake fix.
- **Tags on origin:** `v0.9.0` through `v0.18.0`.
- **CHANGELOG:** `[Unreleased]` populated with the three
  post-v0.18.0 commits — promote to `[0.19.0]` whenever the
  next cut feels right.

## Where we are

v0.18.0's resume-prompt menu (A–D) is mostly closed:

- ~~**A — Tag v0.18.0:**~~ shipped at `bfcd05f`.
- ~~**B — Solver-on-`AsyncComputeTaskPool` for the H-key hint:**~~
  shipped at `3e11e9e`. New module `pending_hint.rs` carries the
  `PendingHintTask` resource and `poll_pending_hint_task` system,
  mirroring the `PendingNewGameSeed` pattern.
- **C — Desktop packaging:** unchanged, still gated on artwork +
  signing certs from the player.
- ~~**D — Persistent share link:**~~ shipped at `42d90b1`.
  `Replay.share_url: Option<String>` (with `#[serde(default)]`),
  Stats overlay's "Copy share link" reads from
  `history.0.replays[selected.0].share_url`,
  `LastSharedReplayUrl` resource removed.

The `auto_save_writes_after_30_seconds` flake has been fixed at
`91b7605` by clearing `PendingRestoredGame` in the test fixture
and re-arming the timer in a small bounded loop until the file
appears. No production-code change.

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

## v0.19.0 candidates ([Unreleased] in CHANGELOG)

| Area | Commit | What landed |
|---|---|---|
| Async H-key hint | `3e11e9e` | New `pending_hint.rs` module: `PendingHintTask` resource, `poll_pending_hint_task` + `drop_pending_hint_on_state_change` systems, cancel-on-replace, stale-state guard via `move_count_at_spawn`. Removes the last synchronous solver hot path. |
| Persistent share URLs | `42d90b1` | `Replay.share_url: Option<String>` with `#[serde(default)]`. `poll_replay_upload_result` writes into `replays[0].share_url` + persists. Stats Copy button reads from the selected replay. `LastSharedReplayUrl` deleted. |
| Auto-save flake fix | `91b7605` | `test_app` clears `PendingRestoredGame(None)` after plugin build (preventing dev-machine `game_state.json` from leaking into tests); `auto_save_writes_after_30_seconds` re-arms the timer in a bounded loop instead of single-frame. No production-code change. |

## Open punch list

### Carried forward

- **Desktop packaging** per `ARCHITECTURE.md §17`. Arch PKGBUILD
  exists in `/home/manage/solitaire-quest-pkgbuild/` (separate
  repo). Pending: app icon, macOS `.icns` + notarisation cert,
  Windows `.ico` + Authenticode cert, AppImage recipe.
- **Per-mode artwork** for the Home picker tiles. Currently
  Unicode glyphs from FiraMono's actual coverage as placeholders
  (♣ ◆ ○ ▲ →). When real artwork lands, swap each tile's `Text`
  node for an `Image` node — tile layout, focus order, click
  handling, and chip rendering are unchanged.

### Possible next-round candidates

- **Cut v0.19.0** — `[Unreleased]` is a coherent three-commit
  bundle (one feature, one persistence enhancement, one test
  hygiene fix). Tag whenever it feels right.
- **Pending hint task on `.before(GameMutation)`** — currently
  `poll_pending_hint_task` runs on `Update` without explicit
  ordering. Won't bite in practice (the result is purely
  visual — no game state mutation), but matches the seed-async
  template precisely.
- **Settings UI for share-link visibility** — once persistent,
  surfacing whether a given replay has a URL on the Prev/Next
  selector caption (e.g. "Replay 3 / 8 \u{2022} Shareable") is a
  natural micro-feature. Two-line change in
  `format_replay_caption`.

### Process notes

- **Test discipline (continuing).** v0.19.0 candidates added 4
  tests across `solitaire_data` + `solitaire_engine`. Each pins
  a real behaviour contract (backwards-compat deserialisation,
  spawn → poll → emit, cancel-on-replace, persist after upload)
  rather than a stdlib / derive round-trip. The async hint port
  removed 2 stale synchronous tests when their behaviours moved
  to the new module.
- **Async port template (worked this round):** the H-key port
  followed `d489e7a`'s `PendingNewGameSeed` shape one-to-one —
  resource holds `Option<Task<...>>` plus snapshot data; spawn
  helper drops any in-flight task before assigning new; poll
  system runs in `Update`; cancel-on-state-change runs `.chain()`-ed
  before poll. Two tests cover happy path + cancel.
- **Persistence migration template:** for purely-additive replay
  fields, `#[serde(default)]` is the cheap migration. Bumping
  `REPLAY_SCHEMA_VERSION` would have wiped every player's rolling
  history (the loader rejects mismatched schema), so additive
  changes should default-deserialise rather than version-bump.

## Resume prompt

```
You are a senior Rust + Bevy developer working on Solitaire Quest.
Working directory: <Rusty_Solitaire clone path on this machine>.
Branch: master. v0.18.0 is tagged. Three commits sit on top:
async H-key hint, persistent replay share URLs, and an
auto-save test flake fix.

State: HEAD at 91b7605 (auto-save flake fix on top of v0.18.0
+ async hint + persistent share URL).

READ FIRST (in order, before doing anything):
  1. SESSION_HANDOFF.md  — this file
  2. CHANGELOG.md        — [Unreleased] holds the v0.19.0 draft
  3. CLAUDE.md           — unified-3.0 rule set
  4. CLAUDE_SPEC.md      — formal architecture spec
  5. ARCHITECTURE.md     — crate responsibilities + data flow
  6. ~/.claude/projects/<this-project>/memory/MEMORY.md
                         — saved feedback / project context
                           (machine-local; may be missing on a
                           fresh machine)

DECISION TO ASK THE PLAYER FIRST:
  A. Cut v0.19.0 — promote [Unreleased] to [0.19.0], tag,
     push. Mechanical close-out.
  B. Desktop packaging — needs artwork + signing certs from the
     player; can't be driven by the agent alone.
  C. Per-mode artwork — replace Home picker tile glyphs with real
     images once art lands.
  D. Smaller polish ideas in the punch list (pending_hint
     ordering hardening, share-link visibility on selector caption).

WORKFLOW NOTES:
  - Use the system git config (already correct: funman300 /
    funman300@gmail.com). The previous handoff's `-c user.name=...`
    workflow was for a different machine.
  - When attributing playtester feedback in commits/docs, use
    "Quat" not "Rhys" (saved feedback memory).
  - Sub-agents stage + verify only; orchestrator commits.
  - Every commit must pass build / clippy / test before pushing.
  - Push to GitHub (origin) via `gh auth setup-git` (already wired
    on this machine after v0.18.0 was cut).

OPEN AT THE START: ask which of A–D. Don't pick unilaterally.
```
