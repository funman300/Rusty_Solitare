# CLAUDE_PROMPT_PACK.md

version: 1.0

---

# 0. GLOBAL INSTRUCTION (prepend to every prompt)

```
You must follow CLAUDE_SPEC.md strictly.

Rules:
- Do not expand scope beyond what is defined
- Do not refactor unrelated code
- Do not introduce new dependencies
- Prefer minimal, surgical changes
- Use existing patterns in the codebase
- Return minimal diffs or changed functions only

Before writing code:
1. List relevant constraints from CLAUDE_SPEC.md
2. Identify risks
3. Then implement
```

---

# 1. FEATURE IMPLEMENTATION

```
# TASK: Feature Implementation

feature: "<name>"

goal:
"<clear outcome>"

scope:
crates: []
systems: []
files: []

non_goals:
- ""

constraints:
- must follow CLAUDE_SPEC.md
- event-driven architecture required
- no blocking operations
- no cross-crate leakage

acceptance_criteria:
- ""
- ""

edge_cases:
- ""

---

## Required Patterns

Use this pattern for systems:
<PASTE EXISTING SYSTEM SNIPPET HERE>

---

## Output Format

intent:
plan:
constraints_used:
risks:

code_changes:
(minimal diffs only)

notes:
```

---

# 2. BUGFIX

```
# TASK: Bug Fix

bug_description:
"<what is broken>"

expected_behavior:
"<correct behavior>"

root_cause_hint (optional):
""

scope:
crates: []
files: []

constraints:
- minimal fix only
- no refactors unless required
- must add regression protection if applicable

---

## Requirements

1. Identify root cause
2. Fix it minimally
3. Preserve all invariants
4. Do not change unrelated logic

---

## Output Format

analysis:
root_cause:
fix_strategy:

code_changes:
(minimal diff)

regression_test (only if high-value):

notes:
```

---

# 3. REFACTOR

```
# TASK: Refactor

target:
"<what is being improved>"

goal:
"<what improves>"

scope:
crates: []
files: []

non_goals:
- no behavior changes
- no new features

constraints:
- must preserve behavior exactly
- must respect crate boundaries
- must not duplicate logic

---

## Refactor Type

- [ ] simplify logic
- [ ] reduce duplication
- [ ] improve readability
- [ ] performance (non-invasive)

---

## Output Format

analysis:
issues_found:

refactor_plan:

code_changes:
(diff only)

verification:
- behavior unchanged: yes/no
- invariants preserved: yes/no

notes:
```

---

# 4. SYSTEM DESIGN (NEW FEATURE)

```
# TASK: System Design

feature:
"<name>"

goal:
"<what problem it solves>"

constraints:
- must fit existing architecture
- must follow plugin + event model
- must not violate crate boundaries

---

## Required Output

design:

components:
- plugins:
- systems:
- events:
- resources:

data_flow:
(step-by-step)

integration_points:
- where it connects to existing systems

risks:
- ""

tradeoffs:
- ""

---

## DO NOT

- write full implementation
- modify unrelated systems
```

---

# 5. NEW BEVY SYSTEM

```
# TASK: Add Bevy System

system_name:
""

trigger:
(event or condition)

reads:
[Resources]

writes:
[Resources]

emits:
[Events]

constraints:
- must be event-driven
- must not directly mutate unrelated state
- must be single responsibility

---

## Output Format

system_signature:

implementation:
(code only)

notes:
```

---

# 6. CORE LOGIC FUNCTION (solitaire_core)

```
# TASK: Core Logic Implementation

function:
"<name>"

goal:
"<what it does>"

rules:
- no IO
- no async
- no Bevy
- deterministic

invariants:
- ""
- ""

errors:
- ""

---

## Output Format

constraints_checked:

implementation:
(code only)

edge_case_handling:

notes:
```

---

# 7. SYNC / MERGE LOGIC

```
# TASK: Sync Logic

goal:
"<what is being merged or synced>"

constraints:
- must be deterministic
- must be idempotent
- must be lossless
- must not delete data

rules:
- counters → max
- times → min
- collections → union

---

## Output Format

analysis:

merge_logic:

code_changes:

invariants_verified:
- deterministic
- idempotent
- lossless

notes:
```

---

# 8. PERFORMANCE OPTIMIZATION

```
# TASK: Optimization

target:
"<what is slow>"

constraints:CLAUDE_WORKFLOW.md
- no behavior change
- no architecture change
- minimal code changes

---

## Output Format

analysis:
bottleneck:

optimization_strategy:

code_changes:

impact_estimate:

notes:
```

---

# 9. TEST GENERATION (STRICT MODE)

```
# TASK: Test Generation

target:
"<function/system>"

reason:
- bugfix | complex logic | invariant protection

constraints:
- no redundant tests
- must test real behavior
- must fail if logic breaks

---

## Output Format

test_cases:
- ""

test_code:

notes:
```

---

# 10. DEBUGGING / INVESTIGATION

```
# TASK: Debug

problem:
"<symptom>"

context:
"<relevant code or system>"

---

## Required Steps

1. List possible causes
2. Narrow down most likely
3. Suggest verification steps
4. Provide minimal fix

---

## Output Format

hypotheses:

most_likely:

verification_steps:

fix:

notes:
```

---

# 11. HARD CONSTRAINT OVERRIDE (RARE)

```
# TASK: Exception Handling

reason:
"<why constraints must be bent>"

requested_exception:
"<rule being broken>"

justification:
"<why unavoidable>"

---

## Output Format

analysis:

alternatives_considered:

final_decision:

risk:
```

---

# 12. STOP CONDITIONS (always append)

```
Stop when:
- acceptance criteria are met
- code is minimal and correct

Do NOT:
- expand scope
- refactor unrelated code
- optimize prematurely
```

---

# END
