<p align="center">
  <h1 align="center">jsonschema-llm</h1>
  <p align="center">
    Convert any JSON Schema into an LLM-compatible structured output schema.<br/>
    Full round-trip: <strong>Schema → Convert → Generate → Rehydrate → Original Shape</strong>
  </p>
  <p align="center">
    <a href="https://github.com/dotslashderek/jsonschema-llm/releases"><img src="https://img.shields.io/badge/status-alpha-orange" alt="Status: Alpha"></a>
    <a href="COMPATIBILITY.md"><img src="https://img.shields.io/badge/compatibility-matrix-blue" alt="Compatibility Matrix"></a>
  </p>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> •
  <a href="#why">Why</a> •
  <a href="#usage">Usage</a> •
  <a href="#algorithm">Algorithm</a> •
  <a href="#rehydration">Rehydration</a> •
  <a href="#providers">Provider Support</a> •
  <a href="ROADMAP.md">Roadmap</a>
</p>

---

## The Problem

You have a JSON Schema. You want an LLM to generate data that conforms to it. But LLM providers only support a **subset** of JSON Schema — and each provider supports a _different_ subset.

| Feature                       | Your Schema | OpenAI Strict | Gemini | Claude |
| ----------------------------- | :---------: | :-----------: | :----: | :----: |
| `$ref`                        |     ✅      |      ❌       |   ✅   |   ❌   |
| `oneOf` / `allOf`             |     ✅      |      ❌       |   ⚠️   |   ❌   |
| `additionalProperties` (maps) |     ✅      |      ❌       |   ✅   |   ❌   |
| Recursive schemas             |     ✅      |      ❌       |   ✅   |   ⚠️   |
| `minimum` / `maximum`         |     ✅      |      ❌       |   ✅   |   ❌   |
| Open-ended `{type: object}`   |     ✅      |      ❌       |   ⚠️   |   ❌   |

You're left hand-converting schemas, losing information, and writing custom parsing code for every project.

## The Solution

`jsonschema-llm` is a **schema-to-schema compiler**. Feed it any JSON Schema (Draft 2020-12 or earlier) and it produces:

1. **A converted schema** — the most faithful LLM-compatible projection possible
2. **A codec** — rehydration metadata to reconstruct the original shape from LLM output

```
┌──────────────┐     ┌──────────────┐     ┌─────────┐     ┌──────────────┐     ┌──────────────┐
│ Your Schema  │────▶│ jsonschema-  │────▶│  LLM    │────▶│  Rehydrator  │────▶│ Original     │
│ (full)       │     │ llm convert  │     │ (any)   │     │  + codec     │     │ Shape        │
└──────────────┘     └──────┬───────┘     └─────────┘     └──────────────┘     └──────────────┘
                            │
                      ┌─────▼─────┐
                      │  Codec    │
                      │ (sidecar) │
                      └───────────┘
```

This is a standalone, deterministic transformer.

---

## Quick Start

### CLI

```bash
# Install
cargo install jsonschema-llm

# Convert a schema for OpenAI Strict (default)
jsonschema-llm convert schema.json -o schema.llm.json --codec codec.json

# Convert for Gemini (relaxed — preserves more features)
jsonschema-llm convert schema.json -o schema.llm.json --target gemini

# Convert in permissive mode (skip strict enforcement)
jsonschema-llm convert schema.json -o schema.llm.json --mode permissive

# Rehydrate LLM output back to the original shape (pass original schema for type coercion)
jsonschema-llm rehydrate output.json --codec codec.json --schema schema.json
```

### Library

<details>
<summary><strong>TypeScript / JavaScript</strong></summary>

```typescript
import { convert, rehydrate } from "jsonschema-llm";

// Convert
const { schema, codec } = convert(mySchema, { target: "openai-strict" });

// Send to OpenAI
const response = await openai.chat.completions.create({
  model: "gpt-4o",
  response_format: {
    type: "json_schema",
    json_schema: { name: "my_schema", schema, strict: true },
  },
  messages: [{ role: "user", content: prompt }],
});

// Rehydrate — maps restored, nulls stripped, JSON strings parsed, types coerced
const original = rehydrate(
  JSON.parse(response.choices[0].message.content),
  codec,
  mySchema,
);
```

</details>

<details>
<summary><strong>Python</strong></summary>

```python
from jsonschema_llm import convert, rehydrate

# Convert
result = convert(my_schema, {"target": "openai-strict"})

# Send to OpenAI
response = client.chat.completions.create(
    model="gpt-4o",
    response_format={"type": "json_schema", "json_schema": {"name": "my_schema", "schema": result["schema"], "strict": True}},
    messages=[{"role": "user", "content": prompt}]
)

# Rehydrate — maps restored, nulls stripped, JSON strings parsed, types coerced
import json
rehydrated = rehydrate(json.loads(response.choices[0].message.content), result["codec"], my_schema)
original = rehydrated["data"]
```

</details>

<details>
<summary><strong>Java</strong></summary>

```java
import dev.jsonschema.llm.SchemaConverter;
import dev.jsonschema.llm.Rehydrator;

// Convert
var result = SchemaConverter.convert(schema, Target.OPENAI_STRICT);
var convertedSchema = result.schema();
var codec = result.codec();

// ... send convertedSchema to your LLM provider ...

// Rehydrate (pass original schema for type coercion)
var original = Rehydrator.rehydrate(llmOutput, codec, schema);
```

</details>

---

<a id="why"></a>

## Why This Exists

### The Structured Output Gap

Every major LLM provider now supports structured output — the ability to constrain generation to conform to a JSON Schema. This is transformative for code generation, data extraction, API integration, and agent tool use.

But there's a catch: **each provider only supports a subset of JSON Schema**, and the subsets are different. OpenAI is the most restrictive (no `$ref`, no `oneOf`, no `allOf`, no maps, no recursion). Gemini is more relaxed. Claude sits somewhere in between.

If your schema uses any of these features — and most real-world schemas do — you can't use structured output directly. You have to manually convert your schema, losing information in the process, and then manually parse the output back.

### Real-World Validation

This algorithm was validated against production-scale JSON Schema definitions including the **OpenAPI 3.1 Specification Schema** — a complex, real-world schema with:

- Discriminated unions (`oneOf` + `discriminator`)
- Maps everywhere (`additionalProperties` patterns for dynamic key-value structures)
- Opaque plugin configurations (`{type: object}` with no properties, `{}` catch-all schemas)
- `allOf` inheritance across multiple definition layers
- Recursive references

The converted schemas were accepted by **OpenAI Strict Mode**. The LLM generated valid, structured output. The rehydrator reconstructed the original shape. Full round-trip, zero manual intervention.

---

<a id="algorithm"></a>

## Algorithm: The 9-Pass Compiler Pipeline

`jsonschema-llm` transforms schemas through 9 ordered passes, each handling a specific incompatibility. The passes are **ordered** (each assumes previous output), **deterministic**, **provider-aware** (passes are skipped/relaxed per target), **mode-aware** (strict vs permissive), and **metadata-preserving** (every lossy change records how to reverse it).

> 📖 **Full specification with examples, merge rules, and design decisions:** [docs/algorithm.md](docs/algorithm.md)

```
┌─────────────────┐
│  Input Schema   │  (JSON Schema Draft 2020-12)
└────────┬────────┘
         │
    ┌────▼─────────────────────────┐
    │ Pass 0: Normalization        │  ✅ Resolve $ref, normalize drafts
    ├──────────────────────────────┤
    │ Pass 1: Composition          │  ✅ Merge allOf into flat objects
    ├──────────────────────────────┤
    │ Pass 2: Polymorphism         │  ✅ oneOf → anyOf
    ├──────────────────────────────┤
    │ Pass 3: Dictionary           │  ✅ Map<K,V> → Array<{key, value}>
    ├──────────────────────────────┤
    │ Pass 4: Opaque Types         │  ✅ {type: object} / {} → {type: string}
    ├──────────────────────────────┤
    │ Pass 5: Recursion            │  ✅ Inline all $ref, break cycles
    ├──────────────────────────────┤
    │ Pass 6: Strict Enforcement   │  ✅ additionalProperties: false, all required
    ├──────────────────────────────┤
    │ Pass 7: Constraint Pruning   │  ✅ Drop unsupported constraints
    ├──────────────────────────────┤
    │ Pass 9: Provider Compat      │  ✅ Pre-flight provider validation
    └────────┬─────────────────────┘
             │
    ┌────────▼────────┐   ┌───────────┐
    │ Converted Schema │   │   Codec   │
    │ (LLM-compatible) │   │ (sidecar) │
    └─────────────────┘   └───────────┘
```

### Pass Summaries

| Pass  | Name               | What It Does                                                                                                                                                                                                     | Lossy?                       |
| ----- | ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------- |
| **0** | Normalization      | Resolves all `$ref` pointers to inline definitions, normalizes syntax across draft versions (e.g. `items` array → `prefixItems`), and detects recursive cycles for later handling.                               | No                           |
| **1** | Composition        | Merges `allOf` sub-schemas into a single flat object — the common "inheritance" pattern. Properties are unioned, required arrays are unioned, conflicting types are intersected.                                 | Partially                    |
| **2** | Polymorphism       | Rewrites `oneOf` → `anyOf`. OpenAI/Claude can't enforce "exactly one matches" during generation; `anyOf` is functionally equivalent and universally supported. Preserves `discriminator` for variant selection.  | No                           |
| **3** | Dictionary         | Converts `Map<String, T>` patterns (`additionalProperties: T`) into arrays of `{key, value}` items. OpenAI requires `additionalProperties: false` on every object. _Skipped for Gemini._                         | Yes — reversed by rehydrator |
| **4** | Opaque Types       | Converts open-ended schemas (`{type: object}` with no properties, `{}`) into `{type: string}` with instructions to produce JSON-encoded strings.                                                                 | Data preserved, UX degraded  |
| **5** | Recursion          | Inlines all remaining `$ref` pointers and breaks recursive cycles at a configurable depth limit (default 3) using dynamic per-branch cycle detection. Strips `$defs` after resolution. _Skipped for Gemini._     | Depth capped                 |
| **6** | Strict Enforcement | Sets `additionalProperties: false`, moves all properties to `required`, and wraps originally-optional properties in `anyOf: [T, {type: null}]`. The "gatekeeper" pass for OpenAI Strict.                         | No                           |
| **7** | Constraint Pruning | Removes unsupported validation keywords per target (e.g. `minimum`, `maxLength`, `format`), normalizes `const` → `enum`, and sorts enum values to put `default` first. Records dropped constraints in the codec. | Validation-only data lost    |
| **9** | Provider Compat    | Pre-flight checks for target-specific constraints (e.g. root must be object, depth budget, enum homogeneity). Returns soft errors — schema is still produced.                                                    | No (read-only)               |

---

<a id="rehydration"></a>

## Rehydration

The codec sidecar file contains enough information to reconstruct the original data shape from LLM output:

| Codec Type           | Forward (Convert)                              | Reverse (Rehydrate)                 |
| -------------------- | ---------------------------------------------- | ----------------------------------- |
| `map_to_array`       | `{a: 1, b: 2}` → `[{key: "a", value: 1}, ...]` | `[{key: "a", value: 1}]` → `{a: 1}` |
| `json_string_parse`  | `{config: {...}}` → `{config: "{...}"}`        | `"{...}"` → `{...}`                 |
| `recursive_inflate`  | Recursive ref → `"{...}"` at depth limit       | `"{...}"` → `{...}` (same as above) |
| `nullable_optional`  | Required field, optional → nullable            | If `null`, remove key entirely      |
| `dropped_constraint` | `minLength: 1` → removed                       | Post-generation validation          |

```python
# Full round-trip example
from jsonschema_llm import convert, rehydrate

result = convert(my_api_schema)
llm_output = call_openai(result["schema"], prompt)

# LLM output has arrays where you had maps, strings where you had objects, nulls everywhere
# Rehydrate fixes all of it:
rehydrated = rehydrate(llm_output, result["codec"], my_api_schema)

# rehydrated["data"] now has:
# - Maps restored: {"X-Rate-Limit": "100"} instead of [{key: "X-Rate-Limit", value: "100"}]
# - Nulls stripped: optional fields that were null are removed entirely
# - JSON strings parsed: plugin configs are proper objects again
```

---

<a id="providers"></a>

## Provider Target Matrix

| Feature                        | OpenAI Strict |      Gemini      |      Claude      |
| ------------------------------ | :-----------: | :--------------: | :--------------: |
| `additionalProperties: false`  |   Required    |     Optional     |   Recommended    |
| All props `required`           |   Required    |     Optional     |   Recommended    |
| `anyOf`                        |      ✅       |        ✅        |        ✅        |
| `oneOf`                        | ❌ → `anyOf`  | ✅ (skip Pass 2) |   ⚠️ → `anyOf`   |
| `allOf`                        |  ❌ → merge   |    ⚠️ → merge    |    ❌ → merge    |
| Recursive `$ref`               |  ❌ → break   | ✅ (skip Pass 5) | ⚠️ → limit depth |
| `additionalProperties: Schema` |  ❌ → array   | ✅ (skip Pass 3) |    ❌ → array    |
| `{type: object}` (opaque)      |  ❌ → string  |   ⚠️ → string    |   ❌ → string    |
| `minimum` / `maximum`          |   ❌ → drop   |  ✅ (preserve)   |    ❌ → drop     |
| `pattern`                      |      ✅       |        ✅        |    ❌ → drop     |

---

## Known Limitations

1. **Recursion**: Recursive schemas are supported up to a configurable depth (default: 3). Deeply nested recursive structures may be truncated.
2. **Mixed-Type Arrays**: Arrays with mixed types (e.g. `[1, "string"]`) may have reduced fidelity in some LLM providers.
3. **Opaque Objects**: Schemas using `type: "object"` without properties are converted to JSON strings to avoid hallucination, requiring rehydration to restore.
4. **Provider Variations**: While OpenAI Strict is fully validated, other providers (Gemini, Claude) have varying degrees of structured output support. The `COMPATIBILITY.md` file tracks granular feature support.

---

## Architecture

```
┌──────────────────────────────────────────────┐
│              jsonschema-llm-core             │
│                 (Rust crate)                  │
│                                              │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐  │
│  │ Converter│  │  Codec   │  │Rehydrator │  │
│  │ (9 pass) │  │ Builder  │  │           │  │
│  └──────────┘  └──────────┘  └───────────┘  │
└──────────────┬───────────────────────────────┘
               │
    ┌──────────┼──────────────────────┐
    │          │                      │
┌───▼───┐ ┌───▼──────┐ ┌────▼─────┐ ┌▼────────┐
│  CLI  │ │TypeScript│ │  Python  │ │  Java   │
│(Rust) │ │  (WASM)  │ │  (PyO3)  │ │(JNI/FFI)│
└───────┘ └──────────┘ └──────────┘ └─────────┘
```

The core library is written in **Rust** using `serde_json::Value` for schema manipulation with recursive descent transformers. Language bindings are shipped as separate crates: [TypeScript via WASM](https://github.com/dotslashderek/jsonschema-llm/issues/38) (✅ shipped), [Python via PyO3](https://github.com/dotslashderek/jsonschema-llm/issues/39) (✅ shipped), and [Java via JNI](https://github.com/dotslashderek/jsonschema-llm/issues/40) (planned).

---

## Project Status

### v0.1 — Core Pipeline ✅

The 9-pass compiler pipeline, rehydrator, codec, and CLI are all implemented and green.

| Component              | Status      | Notes                                                   |
| ---------------------- | ----------- | ------------------------------------------------------- |
| Pass 0: Normalization  | ✅ Complete | `$ref` resolution, cycle detection, draft normalization |
| Pass 1: Composition    | ✅ Complete | `allOf` merge with property/required union              |
| Pass 2: Polymorphism   | ✅ Complete | `oneOf` → `anyOf` rewrite                               |
| Pass 3: Dictionary     | ✅ Complete | Map → Array transpilation with codec                    |
| Pass 4: Opaque Types   | ✅ Complete | Stringification with codec                              |
| Pass 5: Recursion      | ✅ Complete | Dynamic cycle detection, configurable depth limit       |
| Pass 6: Strict Mode    | ✅ Complete | `additionalProperties: false`, nullable optionals       |
| Pass 7: Constraints    | ✅ Complete | Constraint pruning, enum sorting, const→enum            |
| Rehydrator             | ✅ Complete | Full reverse transforms with advisory warnings          |
| Pipeline (`convert()`) | ✅ Complete | Wires all 9 passes with codec accumulation              |
| CLI                    | ✅ Complete | `convert` and `rehydrate` subcommands via `clap`        |

Validated against production-grade schemas including the OpenAPI 3.1 Specification Schema. All accepted by OpenAI Strict Mode with full round-trip rehydration.

### v0.2 — Roadmap

See **[ROADMAP.md](ROADMAP.md)** for the full prioritized roadmap with epic progress, bucket breakdown, and execution order.

| Epic                                                                                | Status         | Progress | Description                                                                |
| ----------------------------------------------------------------------------------- | -------------- | :------: | -------------------------------------------------------------------------- |
| [Core Improvements](https://github.com/dotslashderek/jsonschema-llm/issues/36)      | 🟡 Active      |   75%    | Walker unification, rehydrator decomposition, test hardening, docs cleanup |
| [FFI Facade](https://github.com/dotslashderek/jsonschema-llm/issues/37)             | ✅ Complete    |   100%   | JSON-string bridge API, stable error codes, serde-ready types              |
| [TypeScript / JS (WASM)](https://github.com/dotslashderek/jsonschema-llm/issues/38) | ✅ Complete    |   100%   | `wasm-pack` + `serde-wasm-bindgen`, npm package                            |
| [Python (PyO3)](https://github.com/dotslashderek/jsonschema-llm/issues/39)          | ✅ Complete    |   100%   | `maturin` + `pythonize`, PyPI package                                      |
| [Java (JNI)](https://github.com/dotslashderek/jsonschema-llm/issues/40)             | ⬜ Not started |    0%    | `jni-rs` + JSON string bridge, Maven Central                               |
| [Conformance Suite](https://github.com/dotslashderek/jsonschema-llm/issues/76)      | ⬜ Not started |    0%    | Provider-specific test suites, OpenAPI/AsyncAPI support                    |
| [Test Harness](https://github.com/dotslashderek/jsonschema-llm/issues/115)          | ⬜ Not started |    0%    | Retry logic, known-fail classification, regression tracking                |

---

## License

[Apache License 2.0](LICENSE)

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.
