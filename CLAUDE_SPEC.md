# CLAUDE_SPEC.md

version: 1.0

---

## 0. Global Rules

(Core determinism, panic policy, and event-driven engine constraints live in CLAUDE.md §2.1, §2.3, §3.1. Listed here only when they add information CLAUDE.md doesn't carry.)

rules:

* id: single_source_of_truth
  description: "GameStateResource is the only mutable game state in runtime"

* id: sync_is_additive
  description: "Remote data must never destructively overwrite local data"

---

## 1. Crate Graph

crates:
solitaire_core:
depends_on: [rand, serde, chrono]
forbidden_deps: [bevy, reqwest, tokio, std::fs]

solitaire_sync:
depends_on: [serde, serde_json, uuid, chrono]
role: "shared_types"

solitaire_data:
depends_on: [solitaire_core, solitaire_sync, reqwest, tokio, keyring]
role: "persistence_and_sync"

solitaire_engine:
depends_on: [bevy, kira, solitaire_core, solitaire_data]
role: "runtime_engine"

solitaire_server:
depends_on: [solitaire_sync, axum, sqlx, jsonwebtoken]
role: "backend"

solitaire_app:
depends_on: [solitaire_engine]
role: "entrypoint"

---

## 2. Data Ownership

ownership:
GameState:
owner: solitaire_core
mutable_in: solitaire_engine
access_pattern: "via GameStateResource only"

StatsSnapshot:
owner: solitaire_data

PlayerProgress:
owner: solitaire_data

AchievementRecord:
owner: solitaire_data

SyncPayload:
owner: solitaire_sync

---

## 3. State Transitions

state_machine:
GameState:
transitions:
- action: move_cards
returns: Result<GameState, MoveError>

```
  - action: draw
    returns: Result<GameState, MoveError>

  - action: undo
    returns: Result<GameState, MoveError>

invariants:
  - "52 cards always exist"
  - "no duplicate card IDs"
  - "all cards belong to exactly one pile"
```

---

## 4. Event System

events:

input:
- MoveRequestEvent
- DrawRequestEvent
- UndoRequestEvent
- NewGameRequestEvent

state:
- StateChangedEvent
- GameWonEvent

meta:
- AchievementUnlockedEvent
- SyncCompleteEvent

rules:

* "Input events trigger core logic"
* "Core logic emits state events"
* "UI reacts to state events only"

---

## 5. Sync Contract

sync:

provider_trait:
methods:
- pull() -> SyncPayload
- push(payload) -> SyncResponse

guarantees:
- "non-blocking during gameplay"
- "blocking allowed on exit only"

merge:
rules:
counters: "max"
best_times: "min"
collections: "union"
achievements: "never removed"

```
properties:
  - deterministic
  - idempotent
  - lossless
```

---

## 6. Persistence

storage:

format: json

files:
- stats.json
- progress.json
- achievements.json
- settings.json
- game_state.json

guarantees:
- atomic_write: true
- crash_safe: true

---

## 7. Engine Rules

engine:

mutation_rules:
- "Only GameLogicSystem mutates GameState"
- "UI systems are read-only"

threading:
- "sync runs on AsyncComputeTaskPool"
- "main thread must never block"

plugins:
pattern: "feature_isolation"
communication: "events"

---

## 8. Server Contract

server:

auth:
method: jwt
access_expiry: 24h
refresh_expiry: 30d

endpoints:
- POST /api/auth/register
- POST /api/auth/login
- GET /api/sync/pull
- POST /api/sync/push

limits:
payload_max: 1MB
rate_limit: "10 req/min auth routes"

---

## 9. Achievement System

achievements:

definition_location: solitaire_core
state_location: solitaire_data

types:
- condition_based
- event_driven

rule:
- "achievements cannot be revoked"

---

## 10. Testing Rules

testing:

philosophy:
- "test real failures"
- "avoid redundant tests"

required_coverage:
solitaire_core:
- move_validation
- undo_integrity
- win_detection

```
solitaire_sync:
  - merge_correctness
  - idempotency
```

---

## 11. Prohibited Patterns

(See CLAUDE.md §11 for the canonical forbidden-patterns list.)

---

## 12. Extension Points

extensibility:

sync_backends:
pattern: "implement SyncProvider"

game_modes:
location: solitaire_core::GameMode

plugins:
rule: "new feature = new plugin"

---

## 13. Validation Checklist (for Claude)

validation:

* check: "crate dependency rules respected"
* check: "no panics in core"
* check: "events used for cross-system communication"
* check: "GameState mutations centralized"
* check: "merge function properties preserved"
* check: "no blocking operations in main loop"

---

## 14. Mental Model

model:

layers:
- core
- engine
- data
- server

flow:
- input -> engine -> core -> engine -> ui
- data <-> sync <-> server
