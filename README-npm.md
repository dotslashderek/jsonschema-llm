# jsonschema-llm

Convert JSON Schema to LLM-compatible structured output schemas.

WASM build of [jsonschema-llm](https://github.com/dotslashderek/jsonschema-llm) for Node.js and browser environments.

## Installation

```bash
npm install jsonschema-llm
```

## Usage

### Node.js

```javascript
const { convert, rehydrate } = require("jsonschema-llm");

// Convert a JSON Schema for OpenAI Structured Output
const result = convert(
  {
    type: "object",
    properties: {
      name: { type: "string" },
      age: { type: "integer", minimum: 0 },
    },
    required: ["name", "age"],
  },
  { target: "openai-strict" },
);

console.log(result.schema); // LLM-compatible schema
console.log(result.codec); // Rehydration metadata

// Rehydrate LLM output back to original shape
const rehydrated = rehydrate(llmOutput, result.codec);
console.log(rehydrated.data); // Original shape restored
console.log(rehydrated.warnings); // Constraint violation warnings
```

### Browser / ESM

```javascript
import init, { convert, rehydrate } from "jsonschema-llm/web";

await init();

const result = convert({
  type: "object",
  properties: { name: { type: "string" } },
});
```

## Options

```typescript
interface ConvertOptions {
  target?: "openai-strict" | "gemini" | "claude"; // Default: "openai-strict"
  maxDepth?: number; // Max ref resolution depth (default: 50)
  recursionLimit?: number; // Max recursive type inlining (default: 3)
  polymorphism?: "any-of" | "flatten"; // Polymorphism strategy (default: "any-of")
}
```

## Targets

| Target          | Description                                       |
| --------------- | ------------------------------------------------- |
| `openai-strict` | OpenAI Strict Mode — most restrictive, all passes |
| `gemini`        | Google Gemini — relaxed, some passes skipped      |
| `claude`        | Anthropic Claude — moderate restrictions          |

## TypeScript

Full TypeScript definitions are included. All types are exported:

```typescript
import {
  convert,
  rehydrate,
  ConvertOptions,
  ConvertResult,
  Codec,
  Transform,
  RehydrateResult,
  Warning,
  StructuredError,
} from "jsonschema-llm";
```

## Links

- [GitHub Repository](https://github.com/dotslashderek/jsonschema-llm)
- [API Documentation](https://github.com/dotslashderek/jsonschema-llm#readme)
- [Issue Tracker](https://github.com/dotslashderek/jsonschema-llm/issues)

## License

Apache-2.0
