# CLAUDE.md

version: unified-4.0

---

# 0. Role of This File

This document defines:

* **Execution rules (what Claude must do)**
* **System constraints (what Claude must never violate)**
* **Operational architecture (how code is structured)**

For full system design details:
→ `ARCHITECTURE.md` (authoritative source of truth)

This file overrides all conversational assumptions.

---

# 1. System Architecture (Authoritative Mapping)

## 1.1 Crates

```text id="crate_map"
solitaire_core/    # PURE logic (no IO, no Bevy, deterministic)
solitaire_sync/    # Shared API + merge logic
solitaire_data/    # Persistence + sync client
solitaire_engine/  # Bevy ECS + UI + gameplay orchestration
solitaire_server/  # Axum backend (optional sync layer)
solitaire_wasm/    # WASM bindings for browser-side replay player
solitaire_app/     # Entry binary
assets/            # Runtime assets (except audio + default theme)
```

---

## 1.2 Architecture Source of Truth

* Full system design: `ARCHITECTURE.md`
* This file NEVER redefines system design
* This file ONLY enforces behavior

---

# 2. Hard Global Constraints (NON-NEGOTIABLE)

These override all other instructions.

## 2.1 Core Determinism

* `solitaire_core` MUST:

  * be deterministic
  * be side-effect free
  * never depend on Bevy / IO / async

---

## 2.2 Sync Isolation

* `solitaire_sync`:

  * no Bevy
  * no IO
  * no engine dependencies
* merge logic must be pure functions only

---

## 2.3 Error Policy

* NO `unwrap()`
* NO `panic!()` in runtime/game logic
* Core game state mutations MUST return:

```rust id="err_model"
Result<T, MoveError>
```

* Engine / UI state changes follow ECS patterns (Resources, Events) —
  they do not return `MoveError`
* Use `thiserror`-derived types for any new error enums outside `solitaire_core`

---

## 2.4 Threading Rules

* Sync must run on `AsyncComputeTaskPool`
* NEVER block Bevy main thread

---

## 2.5 Persistence Rules

* atomic writes only:

  * write `.tmp`
  * rename atomically
* no partial state writes allowed

---

## 2.6 Security Rules

* credentials ONLY via `keyring`
* NEVER store secrets in:

  * files
  * logs
  * source code

---

## 2.7 Sync System Rules

* All sync backends implement:

```rust id="sync_trait"
trait SyncProvider
```

* `SyncPlugin` MUST be backend-agnostic
* NEVER match on backend inside ECS systems

---

# 3. Engine Rules (Bevy Layer)

## 3.1 ECS Design

* systems = single responsibility
* cross-system communication = Events (fire-and-forget triggers)
* persistent shared state = Resources (polled every frame or on change)
* per-entity state = Components only

Events and Resources are both valid communication paths — use Events when
the receiver needs to react once; use Resources when the receiver polls
or when multiple systems read the same value (e.g. `SafeAreaInsets`,
`HudVisibility`, `LayoutResource`).

---

## 3.2 Game State Authority

* ONLY `GameStateResource` can mutate game state
* UI systems MUST NOT directly modify core logic

---

## 3.3 UI-First Constraint (CRITICAL)

Every player action MUST:

* have a visible UI control
* NOT rely solely on keyboard shortcuts

Keyboard shortcuts are:
→ optional accelerators only

**Exception — UI chrome gestures:**
Tap-to-toggle visibility of UI chrome (e.g. auto-hiding HUD band) is
permitted without a visible button. The gesture MUST:
* affect only chrome visibility, never game state
* restore chrome automatically when any modal opens
* be purely additive (game remains fully playable with chrome always visible)

---

## 3.4 Layout System

* recompute on `WindowResized`
* recompute on `SafeAreaInsets` changed
* recompute on `HudVisibility` changed
* `compute_layout` MUST accept `hud_visible: bool`; pass `HUD_BAND_HEIGHT`
  when `true`, `0.0` when `false`
* no fixed resolution assumptions

---

# 4. Asset System Rules

## 4.1 Runtime Assets (AssetServer)

Loaded via:

* `CardImageSet`
* `BackgroundImageSet`
* `FontResource`

Includes:

* cards
* backgrounds
* fonts

---

## 4.2 Embedded Assets

Embed via `include_bytes!()` only when ALL of the following are true:

* the asset is small (< 500 KB uncompressed)
* it changes rarely (not user-customisable)
* a missing file would be a hard crash, not a graceful degradation

Currently embedded:
* **Audio** — all `.wav` files in `audio_plugin.rs`
* **Default card theme** — shipped via `embedded://` scheme in `ThemePlugin`

Do NOT embed card face PNGs, background images, or user fonts —
these are loaded via `AssetServer` so art can be swapped without recompile.

---

## 4.3 Test Compatibility Rule

All asset loaders MUST accept:

```rust id="asset_fallback"
Option<Res<AssetServer>>
```

Must degrade gracefully under `MinimalPlugins`.

---

# 5. Code Standards

## 5.1 Error Handling

* use `thiserror`
* no `Box<dyn Error>` in libraries

---

## 5.2 Public API Rules

* prefer `Into<T>` over concrete types
* publicly exported functions, traits, and non-trivial types require doc comments
* simple marker components, newtype wrappers, and internal `pub` items
  used only within the same crate are exempt from doc comment requirements

---

## 5.3 Derive Order

```rust id="derive_order"
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
```

---

## 5.4 Performance Rules

* NO `clone()` in hot paths
* profile before optimizing

---

## 5.5 SQL Rules

* ONLY `sqlx::query!`
* NO raw SQL strings

---

# 6. Build & Verification Rules

These are mandatory before ANY commit.

```bash id="build_rules"
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

---

# 7. Git Workflow Rules

## Commit format

```text id="commit_fmt"
type(scope): description
```

Examples:

* feat(core): add draw-three rules
* fix(engine): correct drag z-order
* test(core): undo boundary cases

---

## Commit conditions

* tests must pass
* clippy must be clean

NEVER commit otherwise

---

# 8. Change Control (ASK BEFORE DOING)

Claude must request confirmation before:

* adding dependencies to `solitaire_core` or `solitaire_sync`
  (engine/server crates may add deps without confirmation)
* modifying `solitaire_sync` types or the `SyncProvider` trait
* changing DB schema (migrations are append-only)
* introducing `unsafe`
* changing the merge strategy in `solitaire_sync::merge`
* changing the `SyncPayload` wire format (breaking change for existing servers)

---

# 9. System Mental Model (IMPORTANT)

```text id="mental_model"
Core (rules + deterministic logic)
    ↓
Engine (Bevy orchestration)
    ↓
Data layer (persistence + sync)
    ↓
Server (optional external system)
```

Core is always the source of truth.

---

# 10. Known Platform Pitfalls

Must always be handled explicitly:

**All platforms**
* Bevy `Time` uses `f32`
* `sqlx::migrate!()` path is crate-relative
* `dirs::data_dir()` may return `None`
* Linux may lack keyring backend — handle `keyring::Error` gracefully

**Android (active target — not stretch)**
* Safe-area insets arrive in frames 1–3 via JNI polling, not at startup;
  UI that depends on them must handle the zero-inset initial state
* Physical pixels ≠ logical pixels: `SafeAreaInsets` values are physical
  (from `WindowInsets` API); divide by `window.scale_factor()` before
  passing to Bevy `Val::Px`
* `adb shell input tap` uses physical pixel coordinates
* FiraMono (bundled font) covers: ASCII, card suits U+2660–2666,
  Arrows U+2190–21FF. It does NOT cover Geometric Shapes (U+25xx) —
  those render as missing-glyph rectangles on Android
* The gesture/navigation bar at the bottom (≈132px physical on common
  devices) is inside the Bevy viewport; use `SafeAreaInsets.bottom` to
  avoid placing interactive elements in that zone
* `HUD_BAND_HEIGHT` is 128px on Android (two-row wrap) vs 64px on desktop;
  layout constants are `#[cfg(target_os = "android")]` gated
* JNI calls must use `attach_current_thread_permanently` — not
  `attach_current_thread` — to avoid detach-on-drop panics

---

# 11. Forbidden Patterns

* game logic inside Bevy systems
* duplication across crates
* blocking async calls in ECS
* insecure credential storage
* bypassing core logic layer
* hardcoded pixel coordinates in layout — always derive from `compute_layout`
* Unicode Geometric Shapes block (U+25xx) in UI text — not in FiraMono
* spawning a second `ModalScrim` while one already exists without first
  dismissing the existing one (use `scrims.is_empty()` guard)
* reading `SafeAreaInsets` physical values directly into `Val::Px` without
  dividing by `window.scale_factor()`

---

# 12. Execution Rules for Claude

When generating code:

1. respect crate boundaries
2. minimize diff size
3. do not expand scope
4. follow existing patterns
5. preserve invariants

If unclear:
→ ask before acting

---

# 13. Relationship to ARCHITECTURE.md

| File            | Role                      |
| --------------- | ------------------------- |
| CLAUDE.md       | execution + constraints   |
| ARCHITECTURE.md | system design truth       |
| Both combined   | full system understanding |

---
# 14. Modal System Conventions

All full-screen overlay panels MUST use the `spawn_modal` / `ModalScrim` pattern
from `solitaire_engine::ui_modal`.

## 14.1 Spawn pattern

```rust
let scrim = spawn_modal(commands, MyScreenMarker, Z_MODAL_PANEL, |card| {
    spawn_modal_header(card, "Title", font_res);
    // ... body nodes ...
    spawn_modal_actions(card, |actions| {
        spawn_modal_button(actions, MyCloseButton, "Done", None,
                           ButtonVariant::Primary, font_res);
    });
});
// Optional: allow clicking the scrim outside the card to dismiss
commands.entity(scrim).insert(ScrimDismissible);
```

## 14.2 Guard rule

Before spawning a new modal, check `scrims: Query<(), With<ModalScrim>>`
and return early if `!scrims.is_empty()` — unless the new modal is
explicitly replacing the current one (despawn first, then spawn).

## 14.3 Safe area

Every `ModalScrim` automatically receives `padding.bottom` equal to the
logical gesture-bar height via `apply_safe_area_to_modal_scrims` in
`SafeAreaInsetsPlugin`. Do not manually add bottom padding to scrim nodes.

## 14.4 Z-ordering

Use `Z_MODAL_PANEL` from `ui_theme` for all modal scrims. Do not use
raw `z_index` values — they drift and cause ordering bugs.

---

# 15. Android Build & Verification

## 15.1 Build command

```bash
cargo apk build --package solitaire_app --lib
adb install -r target/debug/apk/ferrous-solitaire.apk
```

## 15.2 Coordinate system reminder

Device physical: 1080×2400. Bevy logical: 900×2000. Scale factor: 1.20.
`adb shell input tap X Y` takes PHYSICAL coordinates.
To convert from what you see on screen (logical): multiply by 1.20.

## 15.3 Android-specific test checklist

Before shipping any Android build:
- [ ] Safe area insets arrive and shift HUD correctly (check after 3s)
- [ ] All modal Done buttons are above the gesture bar
- [ ] No Geometric Shapes glyphs in UI text
- [ ] HUD band does not overlap the top status bar
- [ ] Touch drag-and-drop works on all pile types

---

# 16. Context Injection System (AUTOMATIC SCOPE FILTER)

## 16.1 Purpose

Before generating any response, Claude MUST construct a **minimal relevant context set**.

This prevents:

* architectural drift
* irrelevant spec loading
* over-engineering
* cross-crate confusion

---

## 16.2 Input Classification Step (MANDATORY)

Every request MUST be classified into exactly one task type:

```text id="task_types"
feature
bugfix
refactor
system_design
bevy_system
core_logic
sync
optimization
test
debug
```

If uncertain → ask clarification.

---

## 16.3 Context Selection Engine

After classification, Claude MUST include ONLY the relevant sections below.

---

## 16.4 Context Map (CORE RULESET)

### feature

Include:

* §2 Hard Global Constraints
* §3 Engine Rules
* ARCHITECTURE.md (crate of target feature only)
* relevant data models (GameState, SyncPayload if needed)

---

### bugfix

Include:

* §2 Hard Global Constraints
* §5 Code Standards
* affected crate boundaries
* relevant system (engine/core/sync only)

---

### refactor

Include:

* §3 Engine Rules
* §5 Code Standards
* §11 Forbidden Patterns
* target crate boundaries

---

### system_design

Include:

* ARCHITECTURE.md (FULL)
* §9 Mental Model
* §1 System Architecture Mapping

---

### core_logic

Include:

* solitaire_core rules only
* GameState model
* MoveError model
* §2.1–2.3 constraints

---

### bevy_system

Include:

* §3 Engine Rules
* ECS rules (Events/Resources/Components)
* UI-first constraint
* relevant plugin system only

---

### sync

Include:

* SyncProvider trait
* merge strategy rules
* solitaire_sync models
* §2.6 Sync Rules

---

### optimization

Include:

* target crate only
* §5.4 Performance Rules
* hot path constraints

---

### test

Include:

* §6 Build Rules
* relevant module
* expected invariants

---

### debug

Include:

* target file/module only
* §2.3 Error Policy
* runtime assumptions relevant to failure

---

## 16.5 Context Compression Rules

Claude MUST obey:

* never include full ARCHITECTURE.md unless system_design
* max 2 crates per response unless explicitly required
* prefer function-level context over file-level context
* exclude unrelated plugins/systems

---

## 16.6 Context Priority Order

When space is limited:

1. Hard Constraints (§2)
2. Target crate rules
3. Data models
4. Only then: architecture snippets

---

## 16.7 “No Context Pollution” Rule

Claude must NOT include:

* unrelated crates
* unrelated plugins
* unused data models
* full architecture dumps
* speculative systems

---

## 16.8 Self-Check Before Execution

Before writing code, Claude MUST verify:

* [ ] Is only relevant context included?
* [ ] Is at least one hard constraint present?
* [ ] Am I touching more than one crate unnecessarily?
* [ ] Am I duplicating ARCHITECTURE.md content?

If any fail → revise context selection.

---

## 16.9 Injection Output Format (Internal Model)

Claude should behave as if it constructed:

```text id="ctx_format"
[SELECTED TASK TYPE]

[MINIMAL REQUIRED RULES]

[MINIMAL ARCHITECTURE SLICES]

[RELEVANT MODELS]

[REQUEST]
```

---

## 16.10 Relationship to ARCHITECTURE.md

* ARCHITECTURE.md = source of truth
* CLAUDE.md = execution constraints
* THIS SECTION = filtering layer between them

---

# END CONTEXT INJECTION SYSTEM
