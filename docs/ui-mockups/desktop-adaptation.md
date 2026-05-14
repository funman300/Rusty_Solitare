# Terminal — Desktop Adaptation Spec

> **Why this exists.** The 24 mockups in this directory are mobile
> (390 × 844 logical, iPhone 14 Pro frame) with one exception
> (`home-menu-desktop.html`). The Stitch project that produced them
> is named "Ferrous Solitaire *Mobile* Redesign" — the mobile-first
> framing was deliberate when the new Android target opened, but
> desktop is still the primary delivery surface. Porting the mobile
> mockups 1:1 would land a 390-px-wide column floating in the middle
> of an 1800 × 1100 window. This file is the rules-based desktop
> companion — apply these adaptations whenever you port a Bevy
> plugin against a mobile mockup in this directory.

## Status

* **Token system.** All tokens (palette, type scale, spacing,
  radii, motion) in `design-system.md` are layout-agnostic and
  apply unchanged on both targets. Do **not** introduce desktop-
  specific token variants — adapt geometry, not tokens.
* **Already adapted in code.** v0.20.0's port is layout-agnostic
  (modal scaffold, toasts, table chrome, card chrome, gameplay-
  feedback, splash cursor). Those surfaces already adapt
  correctly because their Bevy UI nodes use flex / percent /
  stretch sizing rather than fixed pixel widths from the
  mockups.
* **Not yet adapted in code.** Any future plugin port that
  copies layout from a mobile mockup must apply the rules below.

## Viewport assumptions

| Range | Width × height | Source |
|---|---|---|
| Mobile target | 390 × 844 | iPhone 14 Pro logical, Stitch mockup canvas |
| Desktop minimum | 1024 × 600 | Smaller windows degrade to mobile rules |
| Desktop default | ~70 % of monitor | `apply_smart_default_window_size` (since v0.19.0) |
| Desktop typical | 1600 × 900 to 2560 × 1440 | The range we tune for |
| Desktop max | 3840 × 2160 | 4K, with HiDPI scaling already applied |

The "smart default" sizer means a 1080p monitor opens a ~1344 × 756
window, a 1440p monitor opens ~1792 × 1008, a 4K monitor opens
~2688 × 1512. Tune for the 1600–2400 width band as the centre of
the distribution; below 1024 width, fall back to the mobile rules
verbatim.

## Universal adaptation rules

Apply these to every screen unless the per-screen section
overrides them.

### 1. Edge margins

| Mobile | Desktop |
|---|---|
| `margin-edge: 16px` (`SPACE_4`) | `SPACE_5` (24 px) for windows < 1440 wide; `SPACE_6` (32 px) for 1440–2400; `SPACE_7` (48 px) for ≥ 2400 |

Engine: drive from `LayoutResource` based on `Window` size, not a
constant.

### 2. Modal max-width

| Mobile | Desktop |
|---|---|
| `100% - 2 × edge-margin` | `min(720 px, 50 % of viewport)` |

The 720 px cap is already in `ui_modal::spawn_modal`. No code
change needed; this rule documents *why* it's there.

### 3. Vertical content stacks

A mobile screen often stacks `Header → Body → Footer` vertically
to fit a tall narrow column. On desktop, prefer horizontal
distribution where the content allows:

* **Header rows that stack vertically on mobile** (title above
  count above timer) → keep them in one horizontal row on
  desktop.
* **Two-column flex layouts** (e.g. Settings rows: label left,
  control right) — already work on both targets; no change.
* **Cards stacking with `mt-48`-style fixed gaps** — replace with
  flex / percent gaps so the layout breathes.

### 4. Touch-target minimums

Mobile spec mandates 48 dp minimum touch targets. Desktop has no
such floor (mouse precision is finer), but **don't shrink below
mobile's 48 px** for primary actions — keyboard / gamepad focus
rings still need a visible target.

Secondary controls (chip-style toggles, hotkey hints, etc.) can
shrink to `TYPE_BODY` (14 px) text + `SPACE_3` (12 px) padding on
desktop where they were larger on mobile.

### 5. Bottom-anchored elements

Mobile mockups often anchor key controls (action bar, primary CTA,
toast position) to the bottom of the viewport for thumb reach.
Desktop has no thumb-reach concern:

* **Toasts** — keep bottom-anchored (already done in `a137607`),
  the design language is consistent across targets and the
  bottom is still the least-disruptive overlay zone.
* **Action bars** — top of viewport on desktop unless the
  per-screen section says otherwise. The HUD already sits on
  top.
* **Single primary CTA** — modals already right-align in the
  actions row; no change.

### 6. Typography rungs unchanged

Do **not** shift `TYPE_*` tokens up a rung for desktop. The
spec's 14 / 18 / 26 / 40 progression is already calibrated for
the desktop reading distance (60–90 cm). Mobile uses the same
rungs at a closer reading distance (30–40 cm); same physical
angular size on the eye.

### 7. Hotkey hints become full strings

Mobile cells like `▌Esc` — the cursor block plus key letter — can
expand to `[Esc] cancel` style on desktop where horizontal
real-estate is cheap. Drives discoverability of keyboard-only
flows. Optional; only apply where horizontal space exists.

## Per-screen adaptation rules

### Game Table

Mockup: `game-table-mobile.html` (390 × 844).

| Element | Mobile | Desktop |
|---|---|---|
| HUD band | full width, 56 px tall | full width, 48 px tall |
| Foundation row | 4 piles centred, fan-tight | 4 piles centred, **gutter doubled** so the row fills ~50 % of viewport width |
| Stock + waste | left of foundations, stacked | left of foundations, **horizontal pair**: stock on the left, waste to its immediate right (the mobile vertical pair feels cramped on a wide canvas) |
| Tableau row | 7 columns, 4 % gutter | 7 columns, **6 % gutter**, total tableau block ≤ 70 % viewport width |
| Card aspect | 2 : 3 (already in `Layout::card_size`) | unchanged — card aspect is domain |
| Tableau fan | `TABLEAU_FAN_FRAC = 0.25` | unchanged — fan is in card-height units, not viewport units |
| Drag-shadow offset | small | unchanged — pinned to 0 alpha under Terminal anyway |

**Engine impact:** `solitaire_engine/src/layout.rs::compute_layout`
already drives most of this from `Window::size()`. The mobile vs.
desktop difference is the gutter percentages — bake desktop
gutters when window width ≥ 1024.

### Win Summary

Mockup: `win-summary-mobile.html` (390 × 858).

| Element | Mobile | Desktop |
|---|---|---|
| Modal width | 100 % − 2 × edge | **`min(720 px, 50 % viewport)`** (already done by `ui_modal`) |
| Score row | stacked vertically (line per metric) | **3-column grid**: Score / Time / Moves in one row, breakdown rows below in single-line per row |
| Action buttons | full-width stacked (Play Again, Continue, Stats) | **right-aligned action row** — the existing `spawn_modal_actions` already does this on both targets |

**Engine impact:** `solitaire_engine/src/win_summary_plugin.rs`. The
score-breakdown-stagger animation (`MOTION_SCORE_BREAKDOWN_*`) is
unchanged across targets.

### Settings

Mockup: `settings-mobile.html` (390 × 4330 — long scroll).

| Element | Mobile | Desktop |
|---|---|---|
| Modal width | 100 % − 2 × edge | `min(720 px, 50 % viewport)` |
| Sections | full-width labels above stacked controls | **section labels left, control widget right** — already the engine's pattern; no change |
| Long page | scroll the whole modal | **two-column layout**: nav (sections list) on left ~30 %, current section on right ~70 %. Reduces scroll distance on desktop |
| Sliders | full-width on mobile | cap at 320 px on desktop |

**Engine impact:** if a desktop port wants the two-column nav, it's
a `settings_plugin` rewrite. Keep the existing single-column
stacked-modal layout for now — it works on both targets and the
two-column variant is a polish item, not a blocker.

### Help & Controls

Mockup: `help-mobile.html` (390 × 2544).

| Element | Mobile | Desktop |
|---|---|---|
| Modal width | 100 % − 2 × edge | `min(720 px, 50 % viewport)` |
| Section list | one column of `Heading → 2-col rows` | **two columns of section blocks** for windows ≥ 1280 wide; halves vertical scroll distance |
| Hotkey rows | `key | description` 2-col flex | unchanged; 2-col already adapts |

**Engine impact:** `help_plugin`. Single-column on mobile, 2-col
on desktop windows ≥ 1280 wide is a flex-wrap option.

### Pause Menu

Mockup: `pause-menu-mobile.html` (390 × 1768).

Already a small modal; no significant geometry change. Modal
already uses `ui_modal::spawn_modal` which caps width and centres.
No desktop-specific rule.

### Home Menu

Mockup: `home-menu-mobile.html` and `home-menu-desktop.html`
(both already in this directory — desktop variant is the
authoritative reference).

The desktop mockup already specifies the layout. Cross-check it
against the mobile version when porting; differences are
deliberate (more horizontal real-estate, larger primary CTA, the
secondary actions row).

### Splash

Mockup: `splash-mobile.html` (390 × 844).

| Element | Mobile | Desktop |
|---|---|---|
| Full-screen overlay | `inset-0` | unchanged — splash always covers the viewport |
| Cursor block (`▌`) | 96 px JetBrains Mono | unchanged — already done in `cdcadda`. The 96 px size scales fine on desktop because the splash is a brand beat, not a layout-driven element |
| Title `RUSTY SOLITAIRE` | 32 px | scale to 40 px (`TYPE_DISPLAY`) on desktop |
| Subtitle `TERMINAL EDITION` | 12 px | unchanged |
| Boot log lines | 70 % width column | cap at 480 px so the column doesn't stretch on a wide window |
| Progress bar | 100 % − 2 × edge | cap at 720 px |
| Palette swatch row + version footer | bottom-anchored | unchanged; bottom-anchor still reads correctly on desktop |

**Engine impact:** `splash_plugin` already has the cursor block
(`cdcadda`). The boot log / progress bar / palette swatch rows
are the next polish increment when option D is picked up.

### Stats

Mockup: `stats-mobile.html` (390 × 2624).

| Element | Mobile | Desktop |
|---|---|---|
| Modal width | 100 % − 2 × edge | `min(720 px, 50 % viewport)` |
| Big-number cards | 2 × 2 grid | **4 × 1 row** for windows ≥ 1024 wide (the four headline metrics fit in a single horizontal row at desktop scale) |
| Latest-win caption | full-width line | unchanged |
| Replay clip / share row | full-width row | unchanged |

### Profile / Achievements / Theme Picker / Daily Challenge

These follow the **standard modal pattern** (`spawn_modal` with
header / body / actions). They already work on desktop because
`ui_modal` handles modal-width capping. Per-screen tweaks are
small and listed below; no structural changes:

* **Profile** — avatar + level / streak chips can flow into a
  single horizontal row on desktop instead of stacking.
* **Achievements** — 3 × N grid on mobile becomes 4 × N or 5 × N
  on desktop where windows ≥ 1280 wide.
* **Theme Picker** — 2-col grid of theme cards on mobile becomes
  3- or 4-col on desktop.
* **Daily Challenge** — single-column scroll on both; no change.

## Mockup parity gap

The 9 missing-plugin screens (`splash`, `challenge`, `time-attack`,
`weekly-goals`, `leaderboard`, `sync`, `level-up`, `replay-overlay`,
`radial-menu`) have only mobile mockups. When porting any of these
plugins:

1. Read the mobile mockup for content + visual hierarchy.
2. Apply the universal adaptation rules above.
3. Apply the closest matching per-screen rule (e.g. an info modal
   uses the same shape as Win Summary or Stats).
4. **No new layout pattern without explicit user approval.**
   Adapting an existing pattern is in scope; inventing a desktop-
   specific component is design work and should be flagged as such.

## Process notes

* **Smart-default sizer is the layout's source of truth.** Before
  reading the mockup, always re-read `Window::size()` —
  `apply_smart_default_window_size` runs at startup and the
  player can resize freely. Hardcoded breakpoints in plugin code
  should reference the *current* `Window` width via a
  `LayoutResource` lookup, not the launch size.
* **`WindowResized` already drives layout recomputes** (CLAUDE.md
  §3.4). Any per-window-width adaptation in this file should hook
  into the existing recompute path, not a new system.
* **Mobile rules win at narrow desktop windows.** A user dragging
  their desktop window down to 600 px width is closer to the
  mobile use-case than the desktop one. Below 1024 px width,
  apply the mobile rules verbatim.
* **Run on a 4K monitor before declaring a port done.** HiDPI
  scaling routes through Bevy's logical sizing, but visual
  polish (border thickness, motion budgets at high refresh rate)
  is worth eyeballing.
