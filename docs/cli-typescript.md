# TypeScript SDK — CLI Walkthrough

End-to-end guide: convert a JSON Schema, generate a typed TypeScript SDK, and use it in your application.

## Prerequisites

- `json-schema-llm` CLI ([install](../README.md#quick-start))
- Node.js 20+
- npm or pnpm

---

## Step 1: Convert your schema

Convert a JSON Schema to an LLM-compatible form with per-component schemas and a manifest:

```bash
json-schema-llm convert my-schema.json \
  --output-dir ./converted \
  --target openai-strict
```

This produces:

- `manifest.json` — component listing and metadata
- Per-component `schema.json` / `codec.json` pairs

> **Tip**: Use `--target gemini` or `--target claude` for other providers. Use `--mode permissive` to skip strict enforcement.

---

## Step 2: Generate the SDK

```bash
json-schema-llm gen-sdk \
  --language typescript \
  --schema ./converted \
  --package @my-org/petstore-sdk \
  --output ./my-sdk
```

This generates a TypeScript/Node.js package:

```
my-sdk/
├── package.json
├── tsconfig.json
├── README.md
├── .gitignore
├── src/
│   ├── index.ts             # Barrel re-export
│   ├── pet.ts               # Per-component module
│   └── owner.ts
└── schemas/
    ├── $defs/Pet/schema.json
    ├── $defs/Pet/codec.json
    └── ...
```

---

## Step 3: Install and build

```bash
cd my-sdk
npm install
npm run build
```

> **Important**: The SDK must be built (`tsc`) before use — component modules resolve schema files relative to the compiled `dist/` directory.

---

## Step 4: Use the SDK

### Standalone (schema + codec access)

```typescript
import { pet } from "@my-org/petstore-sdk";

// Pre-loaded LLM-compatible schema (lazy, cached)
const llmSchema = pet.schema();

// Rehydration codec (lazy, cached)
const codec = pet.codec();
```

### With @json-schema-llm/engine (full roundtrip)

```bash
npm install @json-schema-llm/engine @json-schema-llm/wasi
```

```typescript
import { pet } from "@my-org/petstore-sdk";
import {
  LlmRoundtripEngine,
  OpenAIFormatter,
  FetchTransport,
} from "@json-schema-llm/engine";

const engine = new LlmRoundtripEngine(
  new OpenAIFormatter(),
  {
    url: "https://api.openai.com/v1/chat/completions",
    model: "gpt-4o",
    headers: { Authorization: "Bearer YOUR_API_KEY" },
  },
  new FetchTransport(),
);

const result = await pet.generate("Generate a pet named Max", engine);

console.log(result.data); // Rehydrated data in original schema shape
console.log(result.isValid); // true if passes JSON Schema validation
console.log(result.warnings); // Rehydration warnings (e.g. dropped nulls)
console.log(result.validationErrors); // JSON Schema validation errors (via ajv)
```

### Using the engine directly (without gen-sdk)

If you don't need a generated SDK — for example, if you're working with a single schema:

```typescript
import {
  LlmRoundtripEngine,
  OpenAIFormatter,
  FetchTransport,
} from "@json-schema-llm/engine";
import { readFileSync } from "node:fs";

const schema = readFileSync("my-schema.json", "utf-8");

const engine = new LlmRoundtripEngine(
  new OpenAIFormatter(),
  {
    url: "https://api.openai.com/v1/chat/completions",
    model: "gpt-4o",
    headers: { Authorization: "Bearer YOUR_API_KEY" },
  },
  new FetchTransport(),
);

const result = await engine.generate(schema, "Generate a pet named Max");

console.log(result.data);
```

> **Note**: The `generate()` method handles schema conversion internally. Use `generateWithPreconverted()` when working with pre-built schemas from `gen-sdk`.

---

## Reference

| CLI Flag                | Description                                                       |
| ----------------------- | ----------------------------------------------------------------- |
| `--language typescript` | Target language                                                   |
| `--schema <dir>`        | Directory with `manifest.json` (output of `convert --output-dir`) |
| `--package <name>`      | npm package name (supports scoped: `@scope/name`)                 |
| `--output <dir>`        | Output directory for generated project                            |
| `--build-tool npm`      | Build tool (default for TypeScript)                               |
| `--git-init`            | Initialize a git repository in output dir                         |

## Node.js WASI Notes

The `@json-schema-llm/wasi` package uses Node.js's built-in WASI runtime (`node:wasi`). You may see an experimental warning:

```
(node:12345) ExperimentalWarning: WASI is an experimental feature and might change at any time
```

This is expected and harmless. To suppress it, run with:

```bash
node --no-warnings your-script.js
```
