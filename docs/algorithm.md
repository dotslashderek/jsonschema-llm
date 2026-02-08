# Schema Down-Conversion Algorithm — Formal Specification

**Session Type**: Problem Solving + New System
**Date**: 2026-02-07
**Council**: T (lead) + G | 4 rounds (C unavailable — upstream CLI issue)

## Problem Statement

We need a formal, deterministic algorithm that converts **any valid JSON Schema** (Draft 2020-12 or earlier) into the most faithful LLM-compatible projection possible. This algorithm becomes the core specification for an open-source project: a standalone schema-to-schema converter implemented as a Rust CLI with WASM/FFI bindings for TypeScript, Java, and Python.

**No existing tool does this.** All prior art (Instructor, Outlines, LangChain, Marvin, Guidance) operates at the code/wrapper level. This is a greenfield standalone schema transformer.

---

## The Algorithm: 8-Pass Compiler Pipeline

> [!IMPORTANT]
> JSON Schema is designed for **validation** (permissive — "is this valid?"). LLM structured output is designed for **generation** (restrictive — "what shape must I produce?"). The algorithm bridges this gap by making all implicit constraints explicit.

The algorithm targets **OpenAI Strict Mode** as the baseline compilation target (most constrained). Other providers (Gemini, Claude) are treated as supersets where specific passes can be relaxed or skipped.

### Pass 0: Schema Normalization

| Property        | Value                                                                                                                                                                     |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Detects**     | `$ref` pointers, draft version differences, `items` (array form), `definitions` keyword                                                                                   |
| **Transforms**  | Resolves all `$ref` to inline definitions; normalizes draft syntax; migrates `items` array → `prefixItems` + `additionalItems` → `items`; renames `definitions` → `$defs` |
| **Lossy**       | No — purely structural                                                                                                                                                    |
| **Ordering**    | Must be **first** — all subsequent passes assume a fully resolved, self-contained schema                                                                                  |
| **Limitations** | Root-relative JSON Pointers only (`#/...`). `$id`/`$anchor` scoped resolution deferred.                                                                                   |

### Pass 1: Composition Compilation (`allOf` Merge)

| Property       | Value                                                                             |
| -------------- | --------------------------------------------------------------------------------- |
| **Detects**    | `allOf` nodes (schema inheritance/mixins)                                         |
| **Transforms** | Merges all `allOf` sub-schemas into a single flat object schema                   |
| **Lossy**      | Partially — complex constraint intersections approximated or dropped              |
| **Ordering**   | After Pass 0 (needs resolved refs); before Pass 2 (polymorphism needs clean objs) |

### Pass 2: Polymorphism Simplification (`oneOf`/`anyOf`)

| Property       | Value                                                       |
| -------------- | ----------------------------------------------------------- |
| **Detects**    | `oneOf` with/without `discriminator`; multi-variant `anyOf` |
| **Transforms** | Rewrites `oneOf` → `anyOf` (OpenAI supports it natively)    |
| **Lossy**      | No — `anyOf` is semantically compatible for generation      |
| **Ordering**   | After Pass 1 (allOf merged); before Pass 3                  |

> [!IMPORTANT]
> **Round 4 Decision — `anyOf` over Flattening.**
> G proved that flattening causes discriminator hallucination (the "kafka listener" bug). OpenAI Strict supports `anyOf` natively. Using `anyOf` means the model **must** pick a variant branch — once it commits to `http`, kafka-only fields are physically excluded from its valid token set. This also eliminates token waste from nullable variant fields and flow phase confusion.

### Pass 3: Dictionary Transpilation (Map → Array)

| Property       | Value                                                                             |
| -------------- | --------------------------------------------------------------------------------- |
| **Detects**    | `{type: object, additionalProperties: Schema}` (Map pattern); `patternProperties` |
| **Transforms** | `Map<K, V>` → `Array<{key: K, value: V}>`. Nested maps → multi-key arrays.        |
| **Lossy**      | Yes — structural shape changes. **Requires codec entry.**                         |
| **Ordering**   | After Pass 2; before Pass 4                                                       |

### Pass 4: Opaque Type Stringification

| Property       | Value                                                                                |
| -------------- | ------------------------------------------------------------------------------------ |
| **Detects**    | `{type: object}` with no `properties`; `{}` (any type)                               |
| **Transforms** | → `{type: string, description: "JSON-encoded string..."}`. **Requires codec entry.** |
| **Ordering**   | After Pass 3 (maps handled); before Pass 6 (strict needs typed everything)           |

### Pass 5: Recursion Breaking

| Property       | Value                                                                       |
| -------------- | --------------------------------------------------------------------------- |
| **Detects**    | Recursive `$ref` cycles identified in Pass 0                                |
| **Transforms** | OpenAI: prune at depth limit → opaque string. Gemini: skip (native support) |
| **Lossy**      | Yes — structure truncated at depth boundary                                 |

### Pass 6: Strict Mode Enforcement

| Property       | Value                                                                               |
| -------------- | ----------------------------------------------------------------------------------- | ----- |
| **Detects**    | All `type: object` nodes                                                            |
| **Transforms** | `additionalProperties: false`, all props `required`, optionals → `anyOf: [T, null]` |
| **Lossy**      | No — `Optional<T>` ≅ `T                                                             | null` |
| **Ordering**   | **Last structural pass** — seals all objects                                        |

### Pass 7: Constraint Pruning & Enum Sorting

| Property       | Value                                                                          |
| -------------- | ------------------------------------------------------------------------------ |
| **Detects**    | `minLength`, `maximum`, `default`, `const`, `not`, `if/then/else`, etc.        |
| **Transforms** | Drop unsupported constraints with codec entries. **Sort enums default-first.** |
| **Lossy**      | Yes for dropped constraints                                                    |

> [!TIP]
> **Round 4 — Enum Default-First Sorting.** Before stripping `default`, reorder `enum` to place the default value at index 0. LLMs bias toward first options when context is weak.

---

## Rehydration Codec

> [!IMPORTANT]
> **Round 4 — JSON Pointers for codec paths.** The converter emits schema-node pointers (e.g., `#/properties/listeners/items`). The rehydrator tracks schema position as it walks the data instance.

```json
{
  "$schema": "https://jsonschema-llm.dev/codec/v1",
  "transforms": [
    { "path": "#/properties/plans", "type": "map_to_array", "keyField": "key" },
    {
      "path": "#/properties/listeners/items/properties/configuration",
      "type": "json_string_parse"
    },
    {
      "path": "#/properties/tags",
      "type": "nullable_optional",
      "originalRequired": false
    }
  ],
  "droppedConstraints": [
    { "path": "#/properties/name", "constraint": "minLength", "value": 1 }
  ]
}
```

---

## Provider Target Matrix

| Feature                        | OpenAI Strict | Gemini           | Claude           |
| ------------------------------ | ------------- | ---------------- | ---------------- |
| `additionalProperties: false`  | Required      | Optional         | Recommended      |
| `anyOf`                        | ✅            | ✅               | ✅               |
| `oneOf`                        | ❌ → `anyOf`  | ✅ (skip Pass 2) | ⚠️ → `anyOf`     |
| `allOf`                        | ❌ → merge    | ⚠️ → merge       | ❌ → merge       |
| Recursive `$ref`               | ❌ → break    | ✅ (skip Pass 5) | ⚠️ → limit depth |
| `additionalProperties: Schema` | ❌ → array    | ✅ (skip Pass 3) | ❌ → array       |
| `{type: object}` (opaque)      | ❌ → string   | ⚠️ → string      | ❌ → string      |
| `minimum`/`maximum`            | ❌ → drop     | ✅ (preserve)    | ❌ → drop        |
| `pattern`                      | ✅            | ✅               | ❌ → drop        |

---

## Rust Architecture

> [!WARNING]
> **G's original suggestion of `Cow<Schema>` was not adopted.** The codebase uses `serde_json::Value` with explicit cloning. The recursive descent transformer pattern is used, but clone-on-write was unnecessary given the pass-by-pass pipeline design.

- **`serde_json::Value`** — All schema manipulation uses `Value` cloning for simplicity
- **Context struct** — e.g., `RefContext` in Pass 0 bundles root schema + config + traversal state
- **Depth Guard** — Configurable limit (default 50). Error on exceeded depth.
- **WASM-first** — Enable running converter in-browser (e.g., Gravitee Console "AI Ready" preview)

> [!NOTE]
> **Considered and deferred: `Cow<Schema>` clone-on-write.** Schema sizes are inherently bounded by LLM context windows — the converted schema must fit in the model's input alongside prompts and context. With practical ceilings around 64KB of schema JSON, clone-on-write would save microseconds on an operation that already runs in milliseconds. The bottleneck is always LLM inference, not the transformer.

---

## v0.1 Implementation Status

| Priority | Pass       | Name                  | Status         |
| -------- | ---------- | --------------------- | -------------- |
| 1        | Pass 6     | Strict Enforcer       | ✅ Implemented |
| 2        | Pass 3     | Dictionary Transpiler | ✅ Implemented |
| 3        | Pass 1     | Composition           | ✅ Implemented |
| 4        | Rehydrator | (Rust)                | ✅ Implemented |
| 5        | Pass 2     | Polymorphism          | ✅ Implemented |
| 6        | Pass 4     | Opaque Fallback       | ✅ Implemented |
| 7        | Pass 0     | Normalization         | ✅ Implemented |
| 8        | Pass 5     | Recursion Breaking    | 🔲 Stub        |
| 9        | Pass 7     | Constraint Pruning    | 🔲 Stub        |
| 10       | Pipeline   | `convert()` wiring    | 🔲 Stub        |

---

## Future Optimizations (v2+)

- **`--polymorphism=flatten`** — Opt-in flattening for edge cases where anyOf fails
- **Optional Grouping** — Bundle 10+ nullable optional fields into `options: {T | null}`
- **Description Namespacing** — Prepend `[HTTP]`/`[Kafka]` to merged variant field descriptions
- **Provider auto-detection** — Infer target from API key or `--provider` flag

---

## Project Structure

**Name**: `jsonschema-llm`

```
jsonschema-llm/
├── crates/
│   └── jsonschema-llm-core/   # Rust core library
│       └── src/
│           ├── lib.rs          # Public API (convert + rehydrate)
│           ├── passes/         # One module per pass (p0–p7)
│           ├── codec.rs        # Codec builder
│           ├── rehydrator.rs   # Reverse transforms
│           └── schema_utils.rs # Shared path/traversal utilities
├── cli/                       # Rust CLI binary (stub)
├── bindings/                  # Language bindings (not yet implemented)
│   ├── typescript/            # WASM (planned)
│   ├── java/                  # JNI (planned)
│   └── python/                # PyO3 (planned)
├── docs/
│   └── algorithm.md           # This specification
└── README.md
```

---

## Validation Results (Proof of Concept)

| Test               | Input                            | Result                                                 |
| ------------------ | -------------------------------- | ------------------------------------------------------ |
| Test Schema        | 2.5KB, Maps+Discriminator+Opaque | ✅ OpenAI strict, 7/7 round-trip checks                |
| Gravitee v4 Schema | 29KB, 1216 lines, production     | ✅ OpenAI strict, 169 codec entries, round-trip passed |

**Council Session Log:**

- Round 1: T+G independent proposals → T synthesis (8-pass pipeline)
- Round 2: POC implementations, OpenAI test schema validation ✅
- Round 3: Full rehydration round-trip on test schema (7/7) + Gravitee v4 ✅
- Round 4: Retrospective — `anyOf` > flattening, enum sorting, JSON Pointer paths, WASM-first, v0.1 priority
