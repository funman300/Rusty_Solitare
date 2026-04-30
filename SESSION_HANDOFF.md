# Solitaire Quest — UX Overhaul Session Handoff

**Last updated:** 2026-04-30 — Phase 3 complete. All 10 steps landed; ready for full smoke-test.

## Where we are

Phase 3 of the UX overhaul brief is **done**. The whole engine has been migrated to the `ui_theme` design-token system + `ui_modal` scaffold. Animation system upgraded. Final literal sweep landed. The work spans 17 commits this session, from the foundation (`e14852c`) through to the final sweep (`54e024c`).

### Design direction (already saved as project memory)

- **Tone:** Balatro — chunky readable type, theatrical hierarchy, satisfying micro-interactions.
- **Palette:** Midnight Purple base (`BG_BASE` `#1A0F2E` → `BG_ELEVATED` `#2D1B69` → `BG_ELEVATED_HI` `#3A2580` → `BG_ELEVATED_TOP` `#482F97`) + Balatro yellow primary accent (`ACCENT_PRIMARY` `#FFD23F`) + warm magenta secondary (`ACCENT_SECONDARY` `#FF6B9D`).
- See [memory/project_ux_overhaul_2026-04.md](.claude/projects/-home-manage-Rusty-Solitare/memory/project_ux_overhaul_2026-04.md) for the full direction.

### Top complaints from the original smoke test — all closed

1. **HUD too cluttered.** ✅ Closed by `73cad7e` — readouts now sit in a 4-tier vertical stack with progressive disclosure of penalty/bonus tiers.
2. **Y/N keyboard prompts feel like debug panels.** ✅ Closed across Confirm, GameOver, Pause, Forfeit, and Settings modals — every prompt now has real Primary/Secondary/Tertiary buttons with hover/press feedback.

## Foundation (done)

- **`solitaire_engine/src/ui_theme.rs`** — every design token: colours, 5-rung typography scale, 4-multiple spacing scale, three radius rungs, monotonically-ordered z-index hierarchy, motion durations with `scaled_duration(speed)` helper.
- **`solitaire_engine/src/ui_modal.rs`** — `spawn_modal` scaffold + `spawn_modal_header` / `spawn_modal_body_text` / `spawn_modal_actions` / `spawn_modal_button` helpers + `ButtonVariant` enum (Primary / Secondary / Tertiary) + `paint_modal_buttons` system. `UiModalPlugin` registered in `solitaire_app/src/main.rs`.

## Commits this session (Phase 3, latest first)

```
54e024c chore(engine): final literal-to-token sweep
3a01318 feat(engine): upgrade animations — curves, scoped settle, deal jitter, cascade rotation
79d3917 chore(data): derive Copy on AnimSpeed
ba019c0 feat(engine): convert SettingsPanel to modal scaffold + Done button
18d7c12 feat(engine): convert OnboardingPlugin to 3-slide modal flow
cb93bd9 fix(engine): pin modals via GlobalZIndex and surface forfeit-no-op toast
6723416 feat(engine): convert PauseScreen to modal + add ForfeitConfirmScreen
afb0879 docs: add SESSION_HANDOFF.md mid-overhaul checkpoint
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

**Test status:** `cargo build --workspace` clean, `cargo clippy --workspace -- -D warnings` clean, **819 tests pass / 0 failed / 8 ignored**.

## Smoke-test checklist

The whole overhaul is on disk. Worth running through once end-to-end:

1. **Run the game.** `cargo run -p solitaire_app --features bevy/dynamic_linking`.
2. **HUD layout** reads as 4 stacked tiers (Score / Mode / Penalty / Selection) with the new midnight-purple palette.
3. **Open every overlay** — `S` (Stats), `A` (Achievements), `P` (Profile), `O` (Settings), `L` (Leaderboard), `M` (Home), `F1` (Help). Each is a centred card on a uniform scrim with a yellow `Done` / `Close` primary button. Hover/press states on every button.
4. **Settings.** Four sections (Audio / Gameplay / Cosmetic / Sync). Body scrolls within the modal on small windows; `Done` button stays fixed at the bottom regardless of scroll. Card-back / Background pickers tint the selected swatch with `STATE_SUCCESS`.
5. **Confirm flow.** Click `New Game` while a game is in progress — the abandon-current-game modal has real Cancel/Confirm buttons. `Y/Enter` and the yellow primary button start a new game; `N/Esc` and the secondary button cancel.
6. **Pause + Forfeit.** Press `Esc` — pause modal shows real Resume / Forfeit buttons. Forfeit button opens a Cancel/Forfeit confirmation modal stacked above the pause modal (z-index ordered correctly via `GlobalZIndex`).
7. **First-run onboarding.** Delete `settings.json` (or set `first_run_complete = false`) — three-slide flow shows: Welcome → How to play → Keyboard shortcuts. Navigate with `Next` / `Back` buttons or `→` / `←` accelerators. `Esc` skips on slide 0.
8. **Animations.**
   - Slide a card to a pile — motion curves through `SmoothSnap` (slight overshoot + settle), not linear lerp.
   - Drop a card on a valid destination — only the moved cards bounce; the rest of the table stays still.
   - Start a new game — deal stagger is no longer mechanically uniform; cards land with subtle ±10% timing variation.
   - Win a game — cascade now uses `Expressive` curve with per-card ±15° Z-rotation, screen shake driven by the new `MOTION_WIN_SHAKE_*` tokens.
9. **Resize the window** — cards still snap, no "snap-back-and-forth" jitter.
10. **Win modal** — restyled with the design tokens: midnight-purple card, yellow `Play Again` button.

## Open follow-ups (not blockers)

- **Home / Help redundancy.** Home is still a kbd-reference modal that mostly duplicates Help. Three options: (1) keep as-is, (2) convert into a true mode launcher (Classic / Daily / Zen / Challenge / Time Attack cards, locked options visibly disabled below level 5), (3) drop entirely now that the action bar covers everything Home does. Worth asking the user which direction they want.
- **Forfeit countdown toast** is now superseded by the Forfeit modal (`6723416`). Confirm the toast path is no longer reachable when smoke-testing.
- **Sub-rung pixel sizes** (1 px borders, 64/80/110/150/160 px fixed widths, 28/36/50 px specific spacings) were intentionally left as literals during the step-10 sweep — they're below the smallest `SPACE_*` rung. If the design system grows a "fine" spacing tier in the future, those become candidates for migration.

## Resume prompt for the next session

```
The Solitaire Quest UX overhaul Phase 3 is complete (HEAD=54e024c).
Read SESSION_HANDOFF.md and CLAUDE.md before doing anything new.

819 tests pass / 0 fail / 8 ignored. Clippy clean.

Next likely directions:
1. Smoke-test the build end-to-end and report regressions (see the
   checklist in SESSION_HANDOFF.md).
2. Decide what to do with the Home modal (kbd ref vs mode launcher
   vs delete).
3. Phase 4 — feature work, sound design, or accessibility, depending
   on user priority.
```
