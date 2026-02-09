<p align="center">
  <h1 align="center">jsonschema-llm</h1>
  <p align="center">
    Convert any JSON Schema into an LLM-compatible structured output schema.<br/>
    Full round-trip: <strong>Schema → Convert → Generate → Rehydrate → Original Shape</strong>
  </p>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> •
  <a href="#why">Why</a> •
  <a href="#usage">Usage</a> •
  <a href="#algorithm">Algorithm</a> •
  <a href="#rehydration">Rehydration</a> •
  <a href="#providers">Provider Support</a>
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

# Rehydrate LLM output back to the original shape
jsonschema-llm rehydrate output.json --codec codec.json
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

// Rehydrate — maps restored, nulls stripped, JSON strings parsed
const original = rehydrate(
  JSON.parse(response.choices[0].message.content),
  codec,
);
```

</details>

<details>
<summary><strong>Python</strong></summary>

```python
from jsonschema_llm import convert, rehydrate

# Convert
schema, codec = convert(my_schema, target="openai-strict")

# Send to OpenAI
response = client.chat.completions.create(
    model="gpt-4o",
    response_format={"type": "json_schema", "json_schema": {"name": "my_schema", "schema": schema, "strict": True}},
    messages=[{"role": "user", "content": prompt}]
)

# Rehydrate
original = rehydrate(json.loads(response.choices[0].message.content), codec)
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

// Rehydrate
var original = Rehydrator.rehydrate(llmOutput, codec);
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

This algorithm was validated against the **Gravitee API Management v4 API Definition** — a 29KB, 1200-line production JSON Schema with:

- Discriminated unions (`oneOf` + `discriminator` for listeners, selectors, flow steps)
- Maps everywhere (`Map<String, Plan>`, `Map<String, Map<String, ResponseTemplate>>`)
- Opaque plugin configurations (`{type: object}` with no properties)
- `allOf` inheritance across multiple definition layers
- Recursive references

The converted schema was accepted by **OpenAI Strict Mode**. The LLM generated a valid, realistic API definition. The rehydrator reconstructed the original shape. Full round-trip, zero manual intervention.

---

<a id="algorithm"></a>

## Algorithm: The 8-Pass Compiler Pipeline

`jsonschema-llm` transforms schemas through 8 ordered passes, each handling a specific incompatibility. The passes are designed to be:

- **Ordered** — each pass assumes the output of previous passes
- **Deterministic** — same input always produces the same output
- **Provider-aware** — passes can be skipped or relaxed based on target
- **Metadata-preserving** — every lossy transformation records how to reverse it

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
    │ Pass 4: Opaque Types         │  ✅ {type: object} → {type: string}
    ├──────────────────────────────┤
    │ Pass 5: Recursion            │  ✅ Inline all $ref, break cycles
    ├──────────────────────────────┤
    │ Pass 6: Strict Enforcement   │  ✅ additionalProperties: false, all required
    ├──────────────────────────────┤
    │ Pass 7: Constraint Pruning   │  ✅ Drop unsupported constraints
    └────────┬─────────────────────┘
             │
    ┌────────▼────────┐   ┌───────────┐
    │ Converted Schema │   │   Codec   │
    │ (LLM-compatible) │   │ (sidecar) │
    └─────────────────┘   └───────────┘
```

### Pass 0: Schema Normalization

**What it does:** Resolves all `$ref` pointers to inline definitions, normalizes syntax across draft versions, and detects recursive cycles.

**Why:** All subsequent passes assume a fully resolved, self-contained schema tree. Without this, every pass would need its own `$ref` resolution logic.

**Example:**

```diff
 // Before
 {
   "properties": {
-    "address": { "$ref": "#/$defs/Address" }
+    "address": {
+      "type": "object",
+      "properties": {
+        "street": { "type": "string" },
+        "city": { "type": "string" }
+      }
+    }
   }
 }
```

**Lossy:** No — purely structural.

---

### Pass 1: Composition Compilation (`allOf` Merge)

**What it does:** Merges `allOf` sub-schemas into a single flat object. This handles the common "inheritance" pattern where a schema extends a base type.

**Why:** Most LLM providers don't support `allOf`. Even those that do often misinterpret it.

**Merge rules:**

- `properties`: union of all property keys
- `required`: union of all required arrays
- Conflicting types: intersect
- `if`/`then`/`else`: dropped with codec annotation

```diff
 // Before                          // After
 {                                  {
-  "allOf": [                         "type": "object",
-    {                                "properties": {
-      "type": "object",               "id": { "type": "string" },
-      "properties": {                 "name": { "type": "string" },
-        "id": { "type": "string" }    "email": { "type": "string" }
-      }                             },
-    },                              "required": ["id", "name", "email"]
-    {                              }
-      "type": "object",
-      "properties": {
-        "name": { "type": "string" },
-        "email": { "type": "string" }
-      },
-      "required": ["name", "email"]
-    }
-  ]
 }
```

**Lossy:** Partially — simple merges are lossless; complex constraint intersections are approximated.

---

### Pass 2: Polymorphism Simplification (`oneOf` → `anyOf`)

**What it does:** Rewrites `oneOf` to `anyOf`. OpenAI, Gemini, and Claude all support `anyOf` for structured output.

**Why:** `oneOf` has "exactly one must match" semantics that LLM providers can't enforce during generation. `anyOf` ("at least one must match") is functionally equivalent for generation — the model picks one branch and generates accordingly.

When a `discriminator` is present, the discriminator field guides the model to the correct variant. With `anyOf`, once the model commits to a variant, fields from other variants are **physically excluded** from its valid token set — preventing hallucination of wrong-variant fields.

```diff
 // Before                              // After
 {                                      {
-  "oneOf": [                           -  "anyOf": [
     { "type": "object", ... },              { "type": "object", ... },
     { "type": "object", ... }               { "type": "object", ... }
   ],                                      ],
   "discriminator": {                      "discriminator": {
     "propertyName": "type"                  "propertyName": "type"
   }                                       }
 }                                      }
```

**Lossy:** No — semantically compatible for generation.

> **Design Decision:** Earlier iterations of this algorithm used "discriminator flattening" — merging all variants into a single object with nullable fields for each variant's properties. This was abandoned after testing showed it causes **discriminator hallucination**: the model sees all variants' fields equally and may pick fields from the wrong variant. `anyOf` prevents this structurally.

---

### Pass 3: Dictionary Transpilation (Map → Array)

**What it does:** Converts `Map<String, T>` patterns (expressed as `{type: object, additionalProperties: T}`) into arrays of `{key: string, value: T}` items.

**Why:** OpenAI Strict requires `additionalProperties: false` on every object. Maps — where keys are dynamic — are fundamentally incompatible. Arrays with explicit key fields preserve the same information in a structure the LLM can generate.

```diff
 // Before: Map<String, String> (e.g., HTTP headers)
-{
-  "type": "object",
-  "additionalProperties": { "type": "string" }
-}

 // After: Array of key-value pairs
+{
+  "type": "array",
+  "items": {
+    "type": "object",
+    "properties": {
+      "key": { "type": "string" },
+      "value": { "type": "string" }
+    },
+    "required": ["key", "value"],
+    "additionalProperties": false
+  }
+}
```

**Codec entry:**

```json
{ "path": "#/properties/headers", "type": "map_to_array", "keyField": "key" }
```

**Lossy:** Yes — structural shape change. Reversed by the rehydrator.

**Provider override:** Gemini supports `additionalProperties` natively — skip this pass for `--target gemini`.

---

### Pass 4: Opaque Type Stringification

**What it does:** Converts open-ended object schemas (`{type: object}` with no `properties`) into `{type: string}` with a description instructing the LLM to produce a JSON-encoded string.

**Why:** These "opaque" objects appear frequently in plugin/configuration systems where the schema is deliberately open-ended. LLM providers can't generate unconstrained objects, but they can generate JSON strings that you parse afterward.

```diff
 // Before
-{ "type": "object", "description": "Plugin configuration" }

 // After
+{
+  "type": "string",
+  "description": "A JSON-encoded string representing the plugin configuration. Parse with JSON.parse() after generation."
+}
```

**Codec entry:**

```json
{ "path": "#/properties/config", "type": "json_string_parse" }
```

**Lossy:** Data preserved (double-encoded), but UX degraded.

---

### Pass 5: Recursion Breaking

**What it does:** Inlines all remaining `$ref` pointers and breaks recursive cycles at a configurable depth limit (`recursion_limit`, default 3) by replacing them with opaque JSON-string placeholders. Uses **dynamic cycle detection** — a per-branch counter tracks how many times each `$ref` target has been expanded. When the counter reaches the limit, the ref is replaced instead of expanded.

**Why:** OpenAI does not support `$ref` at all. Even providers that do can't handle infinite recursion. After this pass, the schema is fully self-contained — no `$ref` nodes, no `$defs` section.

**How it works:**

- Non-recursive (DAG) refs are inlined normally — this is the common case
- Recursive refs expand `recursion_limit` times, then the deepest occurrence becomes an opaque string
- `$defs` are stripped from the root after all refs are resolved or broken
- A safety check ensures no dangling `$ref` nodes exist before cleanup

```diff
 // Before: self-referencing TreeNode
 {
   "$ref": "#/$defs/TreeNode",
   "$defs": {
     "TreeNode": {
       "type": "object",
       "properties": {
-        "value": { "type": "string" },
-        "children": {
-          "type": "array",
-          "items": { "$ref": "#/$defs/TreeNode" }
-        }
       }
     }
   }
 }

 // After (recursion_limit=2): TreeNode inlined twice, third level is opaque
+{
+  "type": "object",
+  "properties": {
+    "value": { "type": "string" },
+    "children": {
+      "type": "array",
+      "items": {
+        "type": "object",
+        "properties": {
+          "value": { "type": "string" },
+          "children": {
+            "type": "array",
+            "items": {
+              "type": "string",
+              "description": "JSON-encoded TreeNode. Parse as JSON after generation."
+            }
+          }
+        }
+      }
+    }
+  }
+}
```

**Codec entry:**

```json
{
  "path": "#/properties/children/items/properties/children/items",
  "type": "recursive_inflate",
  "original_ref": "#/$defs/TreeNode"
}
```

**Lossy:** Data preserved (deep levels are double-encoded), but structural depth is capped.

**Provider override:** Gemini supports recursive schemas natively — skip this pass entirely for `--target gemini`.

---

### Pass 6: Strict Mode Enforcement

**What it does:** Three transformations on every `type: object` node:

1. Set `additionalProperties: false`
2. Move all properties into `required`
3. Wrap originally-optional properties in `anyOf: [OriginalType, {type: null}]`

**Why:** OpenAI Strict Mode requires all three. This is the "gatekeeper" pass — without it, nothing works.

```diff
 // Before: name is required, email is optional
 {
   "type": "object",
   "properties": {
     "name": { "type": "string" },
     "email": { "type": "string" }
   },
-  "required": ["name"]
+  "required": ["name", "email"],
+  "additionalProperties": false
 }

 // email becomes nullable
 "email": {
-  "type": "string"
+  "anyOf": [{ "type": "string" }, { "type": "null" }]
 }
```

**Lossy:** No — `Optional<T>` ≅ `T | null`.

**Codec entry:**

```json
{
  "path": "#/properties/email",
  "type": "nullable_optional",
  "originalRequired": false
}
```

---

### Pass 7: Constraint Pruning & Enum Sorting

**What it does:** Three-phase recursive transformation:

1. **`const` → `enum` normalization** — converts `{"const": X}` to `{"enum": [X]}` for OpenAI/Claude (Gemini preserves `const` natively)
2. **Enum default-first sorting** — moves the `default` value to `enum[0]` before `default` is dropped
3. **Constraint pruning** — removes unsupported keywords per target, emitting `DroppedConstraint` codec entries

**Why:** These constraints are validation-only — LLMs ignore them during generation. Removing them reduces schema size (and therefore token cost). The `default`-first enum sorting leverages LLM bias toward earlier options.

**Provider-specific:**

- **OpenAI:** Drop `minimum`/`maximum`/`exclusiveMinimum`/`exclusiveMaximum`/`minLength`/`maxLength`/`minItems`/`maxItems`/`multipleOf`/`format`/`default`/`not`/`if-then-else` + object/array structural constraints. Keep `enum`, `pattern`.
- **Gemini:** Preserve `minimum`/`maximum`/`exclusiveMinimum`/`exclusiveMaximum`/`minLength`/`maxLength`/`minItems`/`maxItems`. Drop `not`/`if-then-else`/`multipleOf`/`format` + universal drops.
- **Claude:** Drop most constraints including `pattern`. Keep `enum`.

**Codec entry:**

```json
{
  "path": "#/properties/name",
  "type": "dropped_constraint",
  "constraint": "minLength",
  "value": 1
}
```

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

schema, codec = convert(my_api_schema)
llm_output = call_openai(schema, prompt)

# LLM output has arrays where you had maps, strings where you had objects, nulls everywhere
# Rehydrate fixes all of it:
original_shape = rehydrate(llm_output, codec)

# original_shape now has:
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

## Architecture

```
┌──────────────────────────────────────────────┐
│              jsonschema-llm-core             │
│                 (Rust crate)                  │
│                                              │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐  │
│  │ Converter│  │  Codec   │  │Rehydrator │  │
│  │ (8 pass) │  │ Builder  │  │           │  │
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

The core library is written in **Rust** using `serde_json::Value` for schema manipulation with recursive descent transformers. Language bindings (TypeScript via WASM, Python via PyO3, Java via JNI) are planned but not yet implemented.

---

## Project Status

🚧 **Under active development.**

| Component              | Status         | Notes                                                   |
| ---------------------- | -------------- | ------------------------------------------------------- |
| Pass 0: Normalization  | ✅ Complete    | `$ref` resolution, cycle detection, draft normalization |
| Pass 1: Composition    | ✅ Complete    | `allOf` merge with property/required union              |
| Pass 2: Polymorphism   | ✅ Complete    | `oneOf` → `anyOf` rewrite                               |
| Pass 3: Dictionary     | ✅ Complete    | Map → Array transpilation with codec                    |
| Pass 4: Opaque Types   | ✅ Complete    | Stringification with codec                              |
| Pass 5: Recursion      | ✅ Complete    | Dynamic cycle detection, configurable depth limit       |
| Pass 6: Strict Mode    | ✅ Complete    | `additionalProperties: false`, nullable optionals       |
| Pass 7: Constraints    | ✅ Complete    | Constraint pruning, enum sorting, const→enum            |
| Rehydrator             | ✅ Complete    | Full reverse transforms with advisory warnings          |
| Pipeline (`convert()`) | ✅ Complete    | Wires all 8 passes with codec accumulation              |
| CLI                    | ✅ Complete    | `convert` and `rehydrate` subcommands via `clap`        |
| Bindings               | 🔲 Not started | TypeScript (WASM), Python (PyO3), Java (JNI)            |

The algorithm has been validated against:

- A comprehensive test schema (2.5KB) with maps, discriminated unions, opaque types
- The Gravitee API Management v4 API Definition (29KB, 1216 lines, production schema)

Both were accepted by OpenAI Strict Mode and passed full round-trip rehydration tests.

---

## License

[Apache License 2.0](LICENSE)

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.
