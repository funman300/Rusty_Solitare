# Solitaire Quest — UX Overhaul Session Handoff

**Last updated:** 2026-04-30 — Phase 3 complete + Phase 4 in progress (Track B landed on disk, Track G subset in flight via background agent).

## ⚠️ In-progress work at pause time

Smoke-test passed; Phase 4 was started. Pushed HEAD is `534870a`. The working tree has **uncommitted** work that is NOT pushed:

### Track B — window polish (on disk, ready to commit)

- **File:** `solitaire_app/src/main.rs` (+44 lines)
- **What landed:**
  - X11/Wayland WM_CLASS via `Window::name = Some("solitaire-quest".into())`
  - Default position `WindowPosition::Centered(MonitorSelection::Primary)`
  - `install_crash_log_hook()` wraps the default panic hook to also append a `crash.log` next to `settings.json`. Uses `std::time::SystemTime` (no new chrono dep). Falls through silently if the data dir is unavailable.
- **Skipped this round (deferred):**
  - App icon hookup — no artwork asset exists yet; add the loader path when art lands.
  - Persisted window geometry — needs a `Settings` schema migration.
  - F11 fullscreen toggle — already wired in `input_plugin.rs:114`, no change needed.
- **Build status:** `cargo build -p solitaire_app` clean; `cargo clippy -p solitaire_app -- -D warnings` clean.
- **Suggested commit subject:** `feat(app): window polish — class name, centered position, crash-log hook`

### Track G subset — modal open animation + score-change feedback (in flight)

- A **background agent** (`general-purpose`, no worktree) was launched against this turn's tree to:
  - Extend `spawn_modal` in `solitaire_engine/src/ui_modal.rs` with a `ModalEntering` component + `advance_modal_enter` system that animates scrim alpha 0 → `SCRIM` and card scale 0.96 → 1.0 over `MOTION_MODAL_SECS`. Respects `AnimSpeed::Instant` via `scaled_duration`. Animate-OUT path is intentionally out of scope.
  - In `solitaire_engine/src/hud_plugin.rs`, add a `ScorePulse` 1.0→1.1→1.0 readout pulse over `MOTION_SCORE_PULSE_SECS` and a floating "+N" Text2d (only for ≥ +50 jumps) that drifts up ~40 px and fades over `MOTION_SCORE_PULSE_SECS * 2`.
  - Tests for both behaviours.
- **State at pause:** the agent had partial edits in `solitaire_engine/src/ui_modal.rs` (visible via `git status`) — at least one unused-import warning was already surfacing. It had not reported back when this snapshot was taken.
- **Resume options for the next session:**
  1. **Wait for the notification.** The agent runs in background; if Claude Code is still alive, the completion notification will fire.
  2. **Inspect and finish manually.** `git diff solitaire_engine/src/ui_modal.rs solitaire_engine/src/hud_plugin.rs` to see what landed; finish or revert and restart with a tighter prompt.
  3. **Discard and restart.** `git restore solitaire_engine/src/ui_modal.rs solitaire_engine/src/hud_plugin.rs` then relaunch the agent with the prompt below.

### Next-session workflow at pause

1. Verify the workspace builds cleanly with **all** in-flight changes: `cargo build --workspace && cargo clippy --workspace -- -D warnings && cargo test --workspace`. The Track B `main.rs` change is independent — even if Track G is reverted, B compiles on its own.
2. If Track B is clean and Track G is incomplete or broken: commit Track B first using the subject above, then deal with Track G.
3. If both are clean: commit each as a separate landing — one feature per commit per project convention.
4. Use:
   ```
   git -c user.name=funman300 -c user.email=root@vscode.infinity commit -m "<subject>"
   ```
5. Push with `git push origin master` (requires interactive credentials on `git.aleshym.co`).

### Original Track G subset prompt (for relaunch if needed)

The agent's full brief is preserved here verbatim — paste into a fresh agent if the current one is unrecoverable:

```
Two UI/UX polish items from track G. Tree clean at HEAD `534870a`.
Sub-agents CANNOT git commit — stage your work; orchestrator commits.

G1. Modal open animation: extend spawn_modal in ui_modal.rs with a
ModalEntering component + advance_modal_enter system that animates
scrim alpha 0 → SCRIM and card scale 0.96 → 1.0 over MOTION_MODAL_SECS.
Use scaled_duration for AnimSpeed respect; ease-out curve t*(2-t).
Register the system in UiModalPlugin::build. Animate-OUT is OUT of
scope. Add ≥2 tests covering ModalEntering presence on spawn and
removal after duration elapses.

G2. Score-change feedback in hud_plugin.rs: ScorePulse component that
scales the score Text 1.0→1.1→1.0 over MOTION_SCORE_PULSE_SECS using
triangular curve. Plus a floating "+N" Text2d (only for ≥ +50 jumps)
in ACCENT_PRIMARY that drifts up 40 px and fades over
MOTION_SCORE_PULSE_SECS * 2. Add ≥2 tests for floater spawn on +50
and despawn after lifetime, plus ≥1 test that +5 does NOT spawn.

Hard requirements: workspace build + clippy --workspace -- -D warnings
+ test --workspace all green. Touch ONLY ui_modal.rs, hud_plugin.rs,
optionally ui_theme.rs for new tokens (don't think you'll need any).
DO NOT touch solitaire_app/src/main.rs (parallel work).
```

---

## Where we are (Phase 3)

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
You are a senior Rust + Bevy developer working toward a public release
of Solitaire Quest. Working directory: /home/manage/Rusty_Solitare.
Branch: master. Apply that lens to every decision: prefer shipping
quality (polish, packaging, defaults, credits, crash safety) over
greenfield features. If something is half-done, the question is
"finish for v1 or cut for v1?" not "what else can we add?".

State: HEAD=0066ca6. Phase 3 of the UX overhaul is shipped. cargo
build / clippy --workspace -- -D warnings / test --workspace all
green — 819 tests pass / 0 fail / 8 ignored.

READ FIRST (in order, before doing anything):
  1. SESSION_HANDOFF.md  — full state, smoke-test checklist, follow-ups
  2. CLAUDE.md           — hard rules (UI-first, no panics, etc.)
  3. ARCHITECTURE.md §1, §15, §17 — design principles, platform
                                    targets, deployment guide
  4. ~/.claude/projects/-home-manage-Rusty-Solitare/memory/MEMORY.md
                         — saved feedback / project context

GATING SIGNAL — ASK FIRST, DON'T ASSUME:
Before proposing new work, ask: "Did the smoke-test (items 1-10 in
SESSION_HANDOFF.md) pass, or did anything regress?" If a regression
exists, fix it before opening any new thread.

LIKELY NEXT DIRECTIONS — surface for the user to choose, don't pick
unilaterally. All framed through "what does v1 release need?":

  A. Home modal decision (open in SESSION_HANDOFF.md).
     - keep as kbd-reference (duplicates Help — release-blocking
       confusion?)
     - repurpose as mode launcher (Classic / Daily / Zen / Challenge /
       Time Attack cards, locked options below level 5)
     - drop (action bar already covers every action)

  B. Window + release polish — `solitaire_app/src/main.rs:34-48`
     currently sets only title + resolution + min size. For public
     release the window needs:
       - app icon (taskbar / dock / alt-tab) — Bevy `Window::window_icon`
         or platform `set_window_icon`; ship a .png/.ico asset.
       - window class / app id (`Window::name`) so X11/Wayland and
         Windows group taskbar entries correctly.
       - persist size + position across launches (Settings already
         saves to JSON; add `window_geometry` field).
       - F11 (or a Settings toggle) wired to real fullscreen mode.
       - centered default position on first launch (Bevy supports
         `WindowPosition::Centered`).
       - present_mode + vsync verification — make sure Linux/macOS
         don't ship at uncapped 4000 fps.
       - panic hook (`std::panic::set_hook`) that writes a crash
         report next to the save files instead of silently exiting.
       - macOS Info.plist / Windows .ico bundling — ARCHITECTURE.md
         §17 currently only covers server deploy.

  C. Sound-design audit. The scoped settle bounce (3a01318) means
     audio_plugin.rs trigger sites may fire less often than before;
     verify card_place / card_flip / card_invalid still feel right.

  D. Sync flow end-to-end on a real second machine. Server
     scaffolding exists but the register → push → pull → restore-on-
     other-device round trip hasn't been exercised against the new
     Settings sync section.

  E. Achievement unlock completeness. ARCHITECTURE.md §11 lists 18.
     The three hidden ones (speed_and_skill, comeback, zen_winner)
     are most likely to be untested. For release, every advertised
     achievement needs to actually fire.

  F. Release-readiness backlog:
     - README / store-page copy / screenshots
     - LICENSE + third-party credits (xCards art, FiraMono, Bevy)
     - SemVer + a v0.1.0 git tag
     - itch.io / Steam packaging per platform (ARCHITECTURE.md §15)
     - App signing — macOS notarization, Windows Authenticode,
       Linux AppImage
     - Telemetry / crash reporting — opt-in, off by default; or
       confirm we ship without and rely on player reports

  G. UI/UX professional polish — Phase 3 shipped the design system;
     v1 wants the difference between "consistent" and "feels
     intentional":
       - Microcopy pass: every button label, empty state, error
         message, and onboarding line reviewed for voice + clarity.
         Pick one verb per concept ("Done" vs "Close" vs "OK") and
         apply it everywhere.
       - Empty / loading / error states: Leaderboard before any
         scores, Stats before any games, Sync UI before login.
         Today these are likely blank panels.
       - Modal open/close animation: `MOTION_MODAL_SECS` token exists
         in `ui_theme.rs:255` but isn't wired up — modals
         appear/disappear instantly. Add scale-from-0.96 + scrim fade
         per the token's doc comment.
       - Tooltips on HUD readouts and settings labels. Bevy has no
         built-in tooltip; build a small one. Hover a number to learn
         what it counts.
       - Accessibility: verify the AAA-contrast claim on
         `ACCENT_PRIMARY` over `BG_BASE` (ui_theme.rs:65). Confirm
         `AnimSpeed::Instant` disables every new animation (slide
         curve, scoped settle, deal jitter, cascade rotation). Add
         focus rings on `Button` entities for keyboard navigation.
       - Typography choice: FiraMono is one weight, monospace for
         everything. Consider shipping a second proportional face for
         body + headings, keep mono for numerics (HUD score, timer).
         Or commit to mono and lean into the "calm coder" feel — pick
         deliberately and document the decision.
       - Onboarding artwork: the 3 slides are text + buttons. For
         release, stylised illustrations (or simple animated card
         props on each slide) elevate the first-launch feel.
       - Score-change feedback: floating "+N" numbers when score
         jumps; pulse on the readout when value crosses a milestone.
         `MOTION_SCORE_PULSE_SECS` is already a token.
       - Splash / loading screen: today the window goes straight to
         gameplay. A 1-2 second branded splash signals "real game"
         vs "rust prototype".
       - Hit-target audit: every interactive element ≥ 32 px on
         desktop. Settings has 28 px icon buttons (`ICON_BUTTON_PX`
         in settings_plugin.rs); revisit.
       - Win-moment design: the cascade is good; consider a score-
         breakdown reveal, streak callout, "share your time"
         affordance for v1.

WORKFLOW NOTES:
  - Commits use:
      git -c user.name=funman300 -c user.email=root@vscode.infinity commit -m "..."
  - Sub-agents can Edit/Write but CANNOT `git commit`. Brief them to
    stage + verify only; orchestrator commits on their behalf.
    See memory/feedback_agent_commit_limit.md.
  - Remote push needs interactive credentials on git.aleshym.co; the
    user runs `git push origin master` themselves.
  - Every commit must pass build / clippy / test. Pause-and-verify
    is the user's preferred cadence — one feature per commit.

OPEN AT THE START: ask (1) did smoke-test pass, (2) which of A–G to
pursue first. Do not assume.
```
