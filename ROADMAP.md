# jsonschema-llm Roadmap

> **Status:** Alpha · Last updated: 2026-02-12 · Tracking issue: [#100](https://github.com/dotslashderek/jsonschema-llm/issues/100)

---

## Epic Progress

| Epic                                                                               | Status         | Open | Closed | Progress |
| ---------------------------------------------------------------------------------- | -------------- | :--: | :----: | :------: |
| [#39 Python PyO3](https://github.com/dotslashderek/jsonschema-llm/issues/39)       | ✅ Done        |  0   |   5    |   100%   |
| [#77 Strict Mode](https://github.com/dotslashderek/jsonschema-llm/issues/77)       | ✅ Done        |  0   |   4    |   100%   |
| [#36 Core Improvements](https://github.com/dotslashderek/jsonschema-llm/issues/36) | 🟡 Active      |  3   |   9    |   75%    |
| [#40 Java Binding](https://github.com/dotslashderek/jsonschema-llm/issues/40)      | ⬜ Not started |  5   |   0    |    0%    |
| [#76 Conformance Suite](https://github.com/dotslashderek/jsonschema-llm/issues/76) | ⬜ Not started |  10  |   0    |    0%    |
| [#115 Test Harness](https://github.com/dotslashderek/jsonschema-llm/issues/115)    | ⬜ Not started |  4   |   0    |    0%    |

---

## Prioritized Buckets

### 🔴 Bucket 1: Alpha Release Prerequisites

Ship language bindings as `0.1.0-alpha` with honest documentation.

| PR                                    | Issues                     | Effort   | Status               |
| ------------------------------------- | -------------------------- | -------- | -------------------- |
| ~~Ship PR #114~~                      | ~~#41, #95, #112~~         | ~~Done~~ | ✅ Shipped           |
| ~~1a: Alpha docs + default coercion~~ | ~~#121, #123, #125, #126~~ | ~~S~~    | ✅ Shipped (PR #128) |
| ~~1b: Python binding finish~~         | ~~#63, #64~~               | ~~S~~    | ✅ Shipped (PR #130) |
| **1c: Java JNI binding**              | #65, #66, #67, #68, #69    | L        | Queued               |
| **1d: Registry publishing**           | #131                       | S        | Queued               |

### 🟡 Bucket 2: Architectural Fixes (Post-Alpha, Pre-1.0)

Address fundamental robustness gaps.

| PR                                      | Issues        | Effort  | Status               |
| --------------------------------------- | ------------- | ------- | -------------------- |
| ~~2a: Root-cause type-safety~~          | ~~#122~~      | ~~M~~   | ✅ Shipped (PR #129) |
| ~~2b: Recursive transform composition~~ | ~~#120~~      | ~~M-L~~ | ✅ Shipped (PR #129) |
| **2c: Multi-provider validation**       | #124, #83     | L       | Queued               |
| **2d: Conformance suite foundation**    | #78, #79, #87 | M-L     | Queued               |
| **2e: p9 inner-schema strict mode**     | #110          | S       | 🔥 In progress       |

### 🔵 Bucket 3: Core Refactors (Quality)

From Epic #36 — good engineering, not blocking alpha.

| Issues | Title                                         | Effort |
| ------ | --------------------------------------------- | ------ |
| #42    | Decompose rehydrator monolith                 | M      |
| #43    | Unify pass result types                       | S      |
| #45    | Reduce clone-heavy Value manipulation         | L      |
| #73    | TraversalContext struct for locate_data_nodes | S      |

### ⚪ Bucket 4: Test & Quality Infrastructure

| Issues | Title                                           | Effort |
| ------ | ----------------------------------------------- | ------ |
| #47    | Property-based testing with proptest/fuzz       | L      |
| #105   | Code quality fixes from Copilot review          | S      |
| #106   | Pin Python dependency version bounds            | S      |
| #116   | Retry logic for non-deterministic stress tests  | S      |
| #117   | Classify known-fail schemas as expected         | S      |
| #118   | Timeout/retry/error handling for stress harness | S      |
| #119   | Report diffing / regression tracking            | M      |
| #132   | Stress test clients                             | S      |

### 🟣 Bucket 5: Future Conformance (Post-1.0)

| Issues | Title                               | Effort |
| ------ | ----------------------------------- | ------ |
| #80    | OpenAPI 3.0.x + Swagger 2.0         | L      |
| #81    | AsyncAPI 2.x + Kubernetes CRD       | L      |
| #82    | OpenResponses schema target         | M      |
| #84    | COMPATIBILITY.md badge matrix       | S      |
| #85    | Distributable compiled spec schemas | M      |
| #86    | APIs.guru corpus mining             | L      |

---

## Execution Order

```
✅ Ship PR #114 (depth budget, walker unification, coercion, constraints)
✅ PR 1a (alpha docs + default coercion) — PR #128
✅ PR 2a (root-cause type-safety) — PR #129
✅ PR 2b (recursive transform composition) — PR #129
✅ PR 1b (Python binding finish) — PR #130
  → PR 2e (p9 inner-schema strict mode) ← 🔥 YOU ARE HERE
  → PR 1c (Java JNI binding)
  → PR 1d (Registry publishing)
  → 🚀 Alpha Release 0.1.0-alpha
  → PR 2c/2d (multi-provider + conformance)
  → 🎯 Beta / 1.0 path
```

---

## Stress Test Baseline

> CLI full pipeline (convert → OpenAI → rehydrate → validate)
>
> - Stress schemas: **46/53 (86.8%)**
> - Goal: **90%+** via architectural fixes

---

## Recently Shipped

- ~~#63, #64~~ ✅ Python PyO3 binding + contract tests (PR #130)
- ~~#121~~ ✅ Default coercion to nearest valid (PR #128)
- ~~#123~~ ✅ Constraint ordering consistency (PR #128)
- ~~#125~~ ✅ Alpha documentation (PR #128)
- ~~#126~~ ✅ Coercion + constraint integration tests (PR #128)
- ~~#120~~ ✅ Mixed-type container fix (PR #129)
- ~~#122~~ ✅ Recursive rehydrator expansion fix (PR #129)
- ~~#94~~ ✅ Root schema must be `type: object` (PR #108)
- ~~#96~~ ✅ Heterogeneous enums unsupported (PR #108)
- ~~#77~~ ✅ Mode Flag (PR #108)
