<p align="center">
  <h1 align="center">json-schema-llm</h1>
  <p align="center">
    Convert any JSON Schema into an LLM-compatible structured output schema.<br/>
    Full round-trip: <strong>Schema → Convert → Generate → Rehydrate → Original Shape</strong>
  </p>
  <p align="center">
    <a href="https://github.com/dotslashderek/json-schema-llm/releases"><img src="https://img.shields.io/badge/status-alpha-orange" alt="Status: Alpha"></a>
    <a href="COMPATIBILITY.md"><img src="https://img.shields.io/badge/compatibility-matrix-blue" alt="Compatibility Matrix"></a>
  </p>
</p>

<p align="center">
  <a href="#the-problem">Problem</a> •
  <a href="#quick-start-cli">CLI</a> •
  <a href="#generated-sdks">SDKs</a> •
  <a href="#why-this-exists">Why</a> •
  <a href="docs/architecture.md">Architecture</a> •
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

`json-schema-llm` is a **schema-to-schema compiler** — feed it any JSON Schema and it produces a provider-compatible projection plus a **codec** to reconstruct the original shape from LLM output.

```
Your Schema ──▶ json-schema-llm ──▶ LLM ──▶ Rehydrate + codec ──▶ Original Shape
```

---

## Quick Start — CLI

```bash
# Install
cargo install json-schema-llm

# Convert a schema for OpenAI Strict (default)
json-schema-llm convert schema.json -o schema.llm.json --codec codec.json

# Generate a typed SDK from converted schemas
json-schema-llm gen-sdk --language typescript --schema ./output/ --package @my-org/my-sdk --output ./sdk/
json-schema-llm gen-sdk --language python    --schema ./output/ --package my-sdk          --output ./sdk/
json-schema-llm gen-sdk --language java      --schema ./output/ --package com.example.sdk --output ./sdk/

# Full round-trip helpers
json-schema-llm convert schema.json --output-dir ./output/
json-schema-llm rehydrate output.json --codec codec.json --schema schema.json
json-schema-llm list-components schema.json
json-schema-llm extract schema.json --pointer '#/$defs/Address'
```

---

## Generated SDKs

The primary consumer-facing feature. Run `gen-sdk` to produce a typed SDK for any supported language. Each SDK includes a **`Component` enum** for dynamic dispatch across all components, plus direct per-component access for focused work.

```bash
# Generate a Java SDK (also supports: typescript, python, ruby)
json-schema-llm gen-sdk \
  --language java \
  --schema ./converted/ \
  --package com.example.petstore \
  --output ./my-sdk/
```

### Java (recommended path)

**Unified generator** — use the `Component` enum when the target component is dynamic (e.g. from user input or config):

```java
import com.example.petstore.SchemaGenerator;
import com.example.petstore.SchemaGenerator.Component;
import com.jsonschema.llm.engine.*;

var engine = LlmRoundtripEngine.create(
    new OpenAIFormatter(),
    new ProviderConfig("https://api.openai.com/v1/chat/completions", "gpt-4o",
        Map.of("Authorization", "Bearer " + apiKey)),
    new HttpTransport()
);

// Generate any component by enum
var result = SchemaGenerator.generate(Component.PET, "Generate a pet named Max", engine);

// Dynamic lookup from a string (e.g. HTTP request body)
Component component = Component.from("Pet");
var result2 = SchemaGenerator.generate(component, "Generate a Pet", engine);

// With JSON Patch for schema customization
var result3 = SchemaGenerator.generate(Component.PET, "Generate a pet", engine, myPatches);
```

**Focused component access** — import directly when you know exactly which component you need:

```java
import com.example.petstore.Pet;

var result = Pet.generate("Generate a pet named Max", engine);
var schema = Pet.schema();   // LLM-compatible schema
var codec  = Pet.codec();    // Rehydration codec
```

### TypeScript

```typescript
import { Component, generate } from "@my-org/my-sdk";
import {
  LlmRoundtripEngine,
  OpenAIFormatter,
  FetchTransport,
} from "@json-schema-llm/engine";

const engine = new LlmRoundtripEngine(/* ... */);
const result = await generate(
  Component.UserProfile,
  "Generate a user profile",
  engine,
);
```

### Python

```python
from my_sdk.generator import Component, generate

engine = LlmRoundtripEngine(formatter=OpenAIFormatter(), config=ProviderConfig(...), transport=HttpTransport())
result = generate(Component.USER_PROFILE, "Generate a user profile", engine)
```

Step-by-step SDK guides: **[Java](docs/cli-java.md)** · [Python](docs/cli-python.md) · [TypeScript](docs/cli-typescript.md)

---

## Why This Exists

Every major LLM provider now supports structured output — the ability to constrain generation to conform to a JSON Schema. But **each provider only supports a different subset** of JSON Schema, and most real-world schemas use features none of them support natively (`$ref`, `oneOf`, `allOf`, recursive types, maps).

This algorithm was validated against the **OpenAPI 3.1 Specification Schema** — discriminated unions, maps everywhere, recursive references, opaque plugin configurations — and the converted schemas were accepted by OpenAI Strict Mode with full round-trip rehydration. Zero manual intervention.

---

## Low-Level API

<details>
<summary><strong>TypeScript / JavaScript (WASI bindings)</strong></summary>

```typescript
import { SchemaLlmEngine } from "@json-schema-llm/wasi";

const engine = await SchemaLlmEngine.create();
const { schema, codec } = await engine.convert(mySchema, {
  target: "openai-strict",
});

const response = await openai.chat.completions.create({
  model: "gpt-4o",
  response_format: {
    type: "json_schema",
    json_schema: { name: "my_schema", schema, strict: true },
  },
  messages: [{ role: "user", content: prompt }],
});

const original = await engine.rehydrate(
  JSON.parse(response.choices[0].message.content),
  codec,
  mySchema,
);
engine.close();
```

</details>

<details>
<summary><strong>Python (WASI bindings)</strong></summary>

```python
from json_schema_llm import convert, rehydrate

result = convert(my_schema, {"target": "openai-strict"})
# ... call your LLM provider with result["schema"] ...
rehydrated = rehydrate(llm_output, result["codec"], my_schema)
original = rehydrated["data"]
```

</details>

<details>
<summary><strong>Pre-built Fixtures (OpenAPI 3.1)</strong></summary>

46 pre-built schema/codec pairs for the OpenAPI 3.1 specification schema — ready to use without running the conversion pipeline.

```bash
cat 'fixtures/oas31/openai-strict/$defs/operation/schema.json'
json-schema-llm rehydrate llm-output.json \
  --codec 'fixtures/oas31/openai-strict/$defs/operation/codec.json' \
  --schema fixtures/oas31/source/oas31-schema.json
```

See [`fixtures/oas31/README.md`](fixtures/oas31/README.md) for the full component listing.

</details>

---

## Architecture & Algorithm

The core is a **10-pass compiler pipeline** written in Rust, compiled to a single WASM module (`wasm32-wasip1`) that all language bindings embed via their native WASM runtime.

→ **[Full Architecture & Algorithm Specification](docs/architecture.md)**

---

## Project Status & Roadmap

→ **[ROADMAP.md](ROADMAP.md)** — full prioritized roadmap with epic progress and execution order.

All v0.1 components are shipped: 10-pass compiler, rehydrator, WASI binary, language bindings (Go, TypeScript, Python, Java, Ruby, C#), engine layer (TypeScript, Python, Java), and gen-sdk codegen.

---

## License

[Apache License 2.0](LICENSE)

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.
