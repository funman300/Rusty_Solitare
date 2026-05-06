# CLAUDE_WORKFLOW.md

version: 1.0

---

## 0. Overview

This workflow defines a **two-agent system**:

* **Builder Agent** → writes and modifies code
* **Guardian Agent** → enforces architecture + rejects invalid changes

No code is considered valid unless it passes Guardian validation.

---

## 1. Agent Roles

### 1.1 Builder Agent

role: "code_generation"

responsibilities:

* implement features
* refactor code
* generate tests (only when justified)
* follow CLAUDE_SPEC.md

constraints:

* cannot bypass validation
* must declare intent before writing code

output_contract:
must_produce:
- change_summary
- files_modified
- reasoning (short)
- code_diff

---

### 1.2 Guardian Agent

role: "architecture_enforcement"

responsibilities:

* validate against CLAUDE_SPEC.md
* detect violations
* reject or approve changes
* suggest minimal fixes (not full rewrites)

constraints:

* no feature implementation
* no large rewrites
* must be deterministic

output_contract:
must_produce:
- status: APPROVED | REJECTED
- violations[]
- required_fixes[]
- optional_improvements[]

---

## 2. Workflow Pipeline

```text
User Request
    ↓
Builder Agent (proposal + code)
    ↓
Guardian Agent (validation)
    ↓
IF approved → commit
IF rejected → feedback → Builder retry
```

---

## 3. Builder Protocol

### Step 1 — Intent Declaration

Builder MUST start with:

```yaml
intent:
  feature: "<name>"
  crates_touched: []
  systems_affected: []
  risk_level: low|medium|high
```

---

### Step 2 — Plan

```yaml
plan:
  - step: "..."
  - step: "..."
```

---

### Step 3 — Implementation

* Only modify declared crates
* Follow ownership rules
* Use events for cross-system communication

---

### Step 4 — Output

```yaml
change_summary: "..."

files_modified:
  - path: ...
    change: "..."

violations_self_check:
  - none | list

notes: "short reasoning"
```

---

## 4. Guardian Protocol

### Step 1 — Spec Validation

Check against:

* crate boundaries
* mutation rules
* event system usage
* sync guarantees
* forbidden patterns

---

### Step 2 — Invariant Validation

Must verify:

* GameState invariants preserved
* no new panic paths
* no blocking calls in engine
* merge properties unchanged

---

### Step 3 — Output Decision

#### APPROVED

```yaml
status: APPROVED

notes:
  - "no violations"
```

---

#### REJECTED

```yaml
status: REJECTED

violations:
  - id: core_purity_violation
    file: "solitaire_core/src/..."
    reason: "uses std::fs"

required_fixes:
  - "move IO to solitaire_data"

optional_improvements:
  - "simplify event naming"
```

---

## 5. Enforcement Rules

### Hard Fail (automatic rejection)

* core crate uses IO / Bevy / network
* GameState mutated outside GameLogicSystem
* blocking async on main thread
* duplicate logic across crates
* merge function altered incorrectly

---

### Soft Fail (allowed but flagged)

* unnecessary complexity
* redundant tests
* minor architectural drift

---

## 6. Iteration Loop

Max attempts per task: **3**

```text
Attempt 1 → Reject → Fix
Attempt 2 → Reject → Fix
Attempt 3 → Final decision
```

If still failing:
→ escalate to user

---

## 7. Diff Strategy

Builder MUST produce:

* minimal diffs
* no unrelated refactors
* no formatting-only changes

---

## 8. Test Strategy Integration

Builder rules:

* only add tests if:

  * fixing a bug
  * protecting complex logic
  * validating invariants

Guardian rejects:

* redundant tests
* no-op tests

---

## 9. Optional Extensions

### 9.1 Third Agent (Optimizer)

role: performance + cleanup

runs AFTER approval:

* reduce allocations
* simplify logic
* improve ECS scheduling

---

### 9.2 CI Integration

Pipeline:

```text
Builder → Guardian → cargo check → clippy → tests
```

Guardian runs BEFORE compilation to catch structural issues early.

---

## 10. Example Interaction

### Builder

```yaml
intent:
  feature: "undo stack limit fix"
  crates_touched: [solitaire_core]
  risk_level: low
```

```yaml
change_summary: "limit undo stack to 64 entries"

files_modified:
  - solitaire_core/src/game_state.rs

notes: "prevents unbounded memory growth"
```

---

### Guardian

```yaml
status: APPROVED

notes:
  - "respects core constraints"
  - "no invariant violations"
```

---

## 11. Mental Model

* Builder = **creative**
* Guardian = **strict**

Builder explores
Guardian enforces

Neither replaces the other.

---

## 12. Success Criteria

System is working if:

* architectural violations go to ~0
* code stays consistent across features
* refactors become safe
* complexity grows sub-linearly
