# jsonschema-llm Roadmap

> **Status:** Alpha · Last updated: 2026-02-16 · Tracking issue: [#100](https://github.com/dotslashderek/jsonschema-llm/issues/100)

---

## Epic Progress

| Epic                                                                                    | Status         | Open | Closed | Progress |
| --------------------------------------------------------------------------------------- | -------------- | :--: | :----: | :------: |
| [#77 Strict Mode](https://github.com/dotslashderek/jsonschema-llm/issues/77)            | ✅ Done        |  0   |   4    |   100%   |
| [#37 FFI Facade](https://github.com/dotslashderek/jsonschema-llm/issues/37)             | ✅ Done        |  0   |   5    |   100%   |
| [#38 TypeScript WASM](https://github.com/dotslashderek/jsonschema-llm/issues/38)        | ✅ Done        |  0   |   4    |   100%   |
| [#115 Test Harness](https://github.com/dotslashderek/jsonschema-llm/issues/115)         | ✅ Done        |  0   |   7    |   100%   |
| [#148 WASI Wrappers](https://github.com/dotslashderek/jsonschema-llm/issues/148)        | ✅ Done        |  0   |   4    |   100%   |
| [#154 Conformance Fixtures](https://github.com/dotslashderek/jsonschema-llm/issues/154) | ✅ Done        |  0   |   1    |   100%   |
| [#39 Python PyO3](https://github.com/dotslashderek/jsonschema-llm/issues/39)            | 🚨 Retired     |  —   |   —    |    —     |
| [#40 Java JNI](https://github.com/dotslashderek/jsonschema-llm/issues/40)               | 🚨 Retired     |  —   |   —    |    —     |
| [#36 Core Improvements](https://github.com/dotslashderek/jsonschema-llm/issues/36)      | 🟡 Active      |  3   |   9    |   75%    |
| [#76 Conformance Suite](https://github.com/dotslashderek/jsonschema-llm/issues/76)      | ⬜ Not started |  10  |   0    |    0%    |

---

## Prioritized Buckets

### 🔴 Bucket 1: Alpha Release Prerequisites

Ship the library with honest documentation.

| Item                          | Issues                     | Effort   | Status                      |
| ----------------------------- | -------------------------- | -------- | --------------------------- |
| ~~Ship PR #114~~              | ~~#41, #95, #112~~         | ~~Done~~ | ✅ Shipped                  |
| ~~1a: Alpha docs + coercion~~ | ~~#121, #123, #125, #126~~ | ~~S~~    | ✅ Shipped (PR #128)        |
| ~~1b: Python binding finish~~ | ~~#63, #64~~               | ~~S~~    | ✅ Shipped (PR #130)        |
| ~~1c: Java JNI binding~~      | ~~#65~~                    | ~~L~~    | 🚨 Retired → WASI (PR #155) |
| **1d: Registry publishing**   | #131                       | S        | Queued                      |

### 🟡 Bucket 2: Architectural Fixes (Post-Alpha, Pre-1.0)

| Item                                    | Issues        | Effort  | Status               |
| --------------------------------------- | ------------- | ------- | -------------------- |
| ~~2a: Root-cause type-safety~~          | ~~#122~~      | ~~M~~   | ✅ Shipped (PR #129) |
| ~~2b: Recursive transform composition~~ | ~~#120~~      | ~~M-L~~ | ✅ Shipped (PR #129) |
| ~~2e: p9 inner-schema strict mode~~     | ~~#110~~      | ~~S~~   | ✅ Shipped (PR #138) |
| **2c: Multi-provider validation**       | #124, #83     | L       | Queued               |
| **2d: Conformance suite foundation**    | #78, #79, #87 | M-L     | Queued               |

### 🔵 Bucket 3: Core Refactors (Quality)

From Epic #36 — good engineering, not blocking alpha.

| Issues | Title                                         | Effort |
| ------ | --------------------------------------------- | ------ |
| #42    | Decompose rehydrator monolith                 | M      |
| #43    | Unify pass result types                       | S      |
| #45    | Reduce clone-heavy Value manipulation         | L      |
| #73    | TraversalContext struct for locate_data_nodes | S      |

### ⚪ Bucket 4: Test & Quality Infrastructure

| Issues   | Title                                       | Effort | Status               |
| -------- | ------------------------------------------- | ------ | -------------------- |
| ~~#105~~ | ~~Code quality fixes from Copilot review~~  | ~~S~~  | ✅ Shipped (PR #143) |
| ~~#106~~ | ~~Pin Python dependency version bounds~~    | ~~S~~  | ✅ Shipped (PR #143) |
| ~~#116~~ | ~~Retry logic for non-deterministic tests~~ | ~~S~~  | ✅ Shipped (PR #138) |
| ~~#117~~ | ~~Classify known-fail schemas~~             | ~~S~~  | ✅ Shipped (PR #138) |
| ~~#118~~ | ~~Timeout/retry/error handling~~            | ~~S~~  | ✅ Shipped (PR #144) |
| ~~#119~~ | ~~Report diffing / regression tracking~~    | ~~M~~  | ✅ Shipped (PR #144) |
| ~~#132~~ | ~~Stress test clients (Python + Java)~~     | ~~S~~  | ✅ Shipped (PR #145) |
| #47      | Property-based testing with proptest/fuzz   | L      | Queued               |

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
✅ PR 2a/2b (type-safety + recursive composition) — PR #129
✅ PR 1b (Python binding finish) — PR #130
✅ PR 2e (p9 inner-schema strict mode) — PR #138
✅ Java JNI binding (shipped then retired) — PR #142
✅ Test harness complete (#105, #106, #116–#119, #132) — PRs #138, #143, #144, #145
✅ WASI universal binary spike — PR #151
✅ WASI 6-language wrappers — PR #152
✅ Native binding retirement + ABI handshake — PR #155
✅ Conformance fixture execution — PR #156
  → PR 1d (Registry publishing) ← 🔥 YOU ARE HERE
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

- ~~#147, #148, #149, #153~~ ✅ WASI universal binary, 6 language wrappers, native binding retirement (PRs #151, #152, #155)
- ~~#154~~ ✅ Conformance fixture execution for Go + Python (PR #156)
- ~~#132~~ ✅ Python + Java stress test clients (PR #145)
- ~~#118, #119~~ ✅ Timeout/retry/error handling + report diffing (PR #144)
- ~~#105, #106~~ ✅ Stress code quality fixes + dep pinning (PR #143)
- ~~#65~~ ✅ Java C FFI + JNI bindings (PR #142, later retired)
- ~~#110~~ ✅ p9 inner-schema strict mode (PR #138)
- ~~#116, #117~~ ✅ Retry logic + known-fail classification (PR #138)
- ~~#63, #64~~ ✅ Python PyO3 binding + contract tests (PR #130)
- ~~#121, #123, #125, #126~~ ✅ Alpha docs + coercion + constraints (PR #128)
- ~~#120, #122~~ ✅ Mixed-type container fix + recursive rehydrator (PR #129)
- ~~#94, #96, #77~~ ✅ Root object, heterogeneous enums, mode flag (PR #108)
