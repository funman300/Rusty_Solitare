# CLAUDE.md

version: unified-3.0

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
solitaire_app/     # Entry binary
assets/            # Runtime assets (except audio)
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
* All state transitions:

```rust id="err_model"
Result<T, MoveError>
```

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
* communication = Events only
* shared state = Resources only
* per-entity state = Components only

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

---

## 3.4 Layout System

* recompute on `WindowResized`
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

Only audio:

```text id="audio_rule"
include_bytes!()
```

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
* all public items require doc comments

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

* adding dependencies
* modifying `solitaire_sync`
* changing DB schema
* introducing `unsafe`
* changing merge strategy

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

* Bevy `Time` uses `f32`
* `sqlx::migrate!()` path is crate-relative
* `dirs::data_dir()` may return `None`
* Linux may lack keyring backend

---

# 11. Forbidden Patterns

* game logic inside Bevy systems
* duplication across crates
* blocking async calls in ECS
* insecure credential storage
* bypassing core logic layer

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
# 14. Context Injection System (AUTOMATIC SCOPE FILTER)

## 14.1 Purpose

Before generating any response, Claude MUST construct a **minimal relevant context set**.

This prevents:

* architectural drift
* irrelevant spec loading
* over-engineering
* cross-crate confusion

---

## 14.2 Input Classification Step (MANDATORY)

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

## 14.3 Context Selection Engine

After classification, Claude MUST include ONLY the relevant sections below.

---

## 14.4 Context Map (CORE RULESET)

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

## 14.5 Context Compression Rules

Claude MUST obey:

* never include full ARCHITECTURE.md unless system_design
* max 2 crates per response unless explicitly required
* prefer function-level context over file-level context
* exclude unrelated plugins/systems

---

## 14.6 Context Priority Order

When space is limited:

1. Hard Constraints (§2)
2. Target crate rules
3. Data models
4. Only then: architecture snippets

---

## 14.7 “No Context Pollution” Rule

Claude must NOT include:

* unrelated crates
* unrelated plugins
* unused data models
* full architecture dumps
* speculative systems

---

## 14.8 Self-Check Before Execution

Before writing code, Claude MUST verify:

* [ ] Is only relevant context included?
* [ ] Is at least one hard constraint present?
* [ ] Am I touching more than one crate unnecessarily?
* [ ] Am I duplicating ARCHITECTURE.md content?

If any fail → revise context selection.

---

## 14.9 Injection Output Format (Internal Model)

Claude should behave as if it constructed:

```text id="ctx_format"
[SELECTED TASK TYPE]

[MINIMAL REQUIRED RULES]

[MINIMAL ARCHITECTURE SLICES]

[RELEVANT MODELS]

[REQUEST]
```

---

## 14.10 Relationship to ARCHITECTURE.md

* ARCHITECTURE.md = source of truth
* CLAUDE.md = execution constraints
* THIS SECTION = filtering layer between them

---

# END CONTEXT INJECTION SYSTEM
