# Solitaire Quest — UX Overhaul Session Handoff

**Last updated:** 2026-04-30 — paused mid-overhaul to push and let the user smoke-test before resuming.

## Where we are

Phase 3 of the UX overhaul brief in `docs/UX_OVERHAUL_BRIEF.md` (or the inline brief used to kick off the work). 11 commits landed this session — the foundation, the HUD, and every read-only overlay have been migrated to the new design system. Pause / Settings / Onboarding modals, animation upgrades, and the final literal sweep are still pending.

### Design direction (already saved as project memory)

- **Tone:** Balatro — chunky readable type, theatrical hierarchy, satisfying micro-interactions.
- **Palette:** Midnight Purple base (`BG_BASE` `#1A0F2E` → `BG_ELEVATED` `#2D1B69` → `BG_ELEVATED_HI` `#3A2580` → `BG_ELEVATED_TOP` `#482F97`) + Balatro yellow primary accent (`ACCENT_PRIMARY` `#FFD23F`) + warm magenta secondary (`ACCENT_SECONDARY` `#FF6B9D`).
- See [memory/project_ux_overhaul_2026-04.md](.claude/projects/-home-manage-Rusty-Solitare/memory/project_ux_overhaul_2026-04.md) for the full direction.

### Top complaints from the original smoke test

1. **HUD too cluttered.** ✅ Closed by `73cad7e` — readouts now sit in a 4-tier vertical stack with progressive disclosure of penalty/bonus tiers.
2. **Y/N keyboard prompts feel like debug panels.** ✅ Closed for the Confirm + GameOver modals (`3f922ed`, `242b5fe`); still open for the Forfeit toast countdown (folded into step 6 below).

## Foundation (done)

- **`solitaire_engine/src/ui_theme.rs`** — every design token: colours, 5-rung typography scale, 4-multiple spacing scale, three radius rungs, monotonically-ordered z-index hierarchy, motion durations with `scaled_duration(speed)` helper.
- **`solitaire_engine/src/ui_modal.rs`** — `spawn_modal` scaffold + `spawn_modal_header` / `spawn_modal_body_text` / `spawn_modal_actions` / `spawn_modal_button` helpers + `ButtonVariant` enum (Primary / Secondary / Tertiary) + `paint_modal_buttons` system. `UiModalPlugin` registered in `solitaire_app/src/main.rs`.

Every overlay conversion follows the same pattern: drop the bespoke scrim/card, call `spawn_modal(MarkerComponent, Z_MODAL_PANEL, |card| { ... })`, swap inline colours for tokens, replace "Press X to close" prose with a primary `Done` button + matching click-handler system.

## Commits this session

```
3b619b8 feat(engine): convert HomeScreen to modal scaffold + Done button
37681cf feat(engine): convert LeaderboardScreen to modal scaffold + Done button
99064ce feat(engine): convert ProfileScreen to modal scaffold + Done button
de4dba6 feat(engine): convert AchievementsScreen to modal scaffold + Done button
75fc3aa feat(engine): convert StatsScreen to modal scaffold + Done button
deb034c feat(engine): convert HelpScreen to real-button modal with kbd-chip rows
242b5fe feat(engine): convert GameOverScreen to real-button modal
3f922ed feat(engine): convert ConfirmNewGameScreen to real-button modal
8da62bd feat(engine): add ui_modal primitive (scaffold + button variants)
73cad7e feat(engine): restructure HUD into 4-tier layout, adopt design tokens
e14852c feat(engine): add ui_theme.rs design-token module
```

**Test status across every commit:** `cargo build --workspace` clean, `cargo clippy --workspace -- -D warnings` clean, **797 tests pass / 0 failed / 8 ignored**.

## What's still on disk in pre-overhaul style

| Step | Area | What it'll touch |
|---|---|---|
| 6 | Pause overlay + Forfeit modal | `pause_plugin.rs` (replace bespoke spawn with modal scaffold); fold Forfeit toast countdown into a real Cancel/Forfeit modal — Forfeit then becomes a button inside Pause. Tests around `forfeit_countdown` will need updating. |
| 7 | Settings panel | `settings_plugin.rs` — replaces the side-panel pattern with the modal scaffold. Sections: Audio / Gameplay / Cosmetic / Sync. The existing per-button click handlers stay; only the spawn structure and styling change. |
| 8 | Onboarding | `onboarding_plugin.rs` — multi-slide flow (Welcome → Drag-drop demo → Hotkeys), navigated by a primary `Next` button + arrow-key accelerators. Final slide `Start playing` writes `first_run_complete` like today. |
| 9 | Animation upgrades | `animation_plugin.rs`, `feedback_anim_plugin.rs`, `card_animation/`, `win_summary_plugin.rs` — promote `card_animation::MotionCurve` to default for slide; settle bounce only on the moved card; deal stagger ±10% jitter; win cascade with `Expressive` curve + per-card rotation; align all durations through `ui_theme::scaled_duration`. |
| 10 | Literal-to-token sweep | Grep the engine for any remaining `Color::srgb`/`Val::Px(<literal>)`/font-size literal and migrate. Clippy gates this — anything left over surfaces during the build. |

ETA for the rest: roughly 5–7 more focused commits, each independently passing `build` / `clippy` / `test`.

## Smoke-test checklist before resuming

The user smoke-tested at the action-bar checkpoint and reported "nothing looks off". Worth re-running once these 11 commits are pushed so the next session starts from a verified baseline:

- HUD layout reads as 4 stacked tiers (Score / Mode / Penalty / Selection) with the new midnight-purple palette.
- Open every overlay (S, A, P, O, L, M, F1) — each should now be a centred card on a uniform scrim, with a `Done` primary button.
- Click `New Game` while a game is in progress — the abandon-current-game modal should be a real-button card; both `Y/Enter` and the yellow primary button start a new game; both `N/Esc` and the secondary button cancel.
- Force a stuck state — the Game Over modal should show real Undo / New Game buttons.
- Resize the window — cards still snap, no "snap-back-and-forth" jitter (covered by previous fixes `366fd6d` / `b10e1a5`).
- Press G to forfeit — still uses the toast countdown (step 6 will replace this with a modal).

## Kickoff prompt for the next session

Paste this at the start of a fresh chat to resume:

```
Resume the UX overhaul of Solitaire Quest at /home/manage/Rusty_Solitare/.

Read these in order before doing anything:
  1. SESSION_HANDOFF.md (this file)
  2. CLAUDE.md (project conventions; UI-first principle)
  3. The "Design direction" section of
     ~/.claude/projects/-home-manage-Rusty-Solitare/memory/project_ux_overhaul_2026-04.md

The foundation (ui_theme.rs, ui_modal.rs), the HUD restructure, and
all 8 read-only overlay conversions are committed. Steps 6 → 10 are
documented in SESSION_HANDOFF.md and listed in
.claude/projects/.../memory/MEMORY.md.

Pick up at step 6 (Pause modal + folded-in Forfeit modal). Same
shape as the existing modal conversions: spawn_modal scaffold, real
buttons, hotkey-hint chips, ui_theme tokens for every colour /
spacing / typography decision. Update or replace any test that
asserted the prior bespoke layout.

Each commit must pass `cargo build --workspace`,
`cargo clippy --workspace -- -D warnings`, and `cargo test --workspace`
clean. Use commits authored by funman300 <root@vscode.infinity> via
the -c flag (per the user's convention; never write to git config).

Stop after step 6 and ask the user to smoke-test before continuing to
step 7. They prefer pause-and-verify over running through the
remaining 4 steps in one push.
```

## Open question

When the next session resumes, ask whether the user wants to:

1. **Keep Home as a kbd-reference modal** (current state — duplicates Help).
2. **Convert Home into a true mode launcher** (Classic / Daily / Zen / Challenge / Time Attack cards, locked options visibly disabled below level 5) per the original Phase 2 proposal. The action-bar `Modes` popover already covers this path; pivoting Home buys discoverability for first-time players who hit `M`.
3. **Drop Home entirely.** With the action bar covering every action and Help covering every shortcut, Home may be redundant.
