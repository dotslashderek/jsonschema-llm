/**
 * TypeScript SDK DX Test — full roundtrip using the generated Arazzo SDK
 * and the @json-schema-llm/engine against Azure OpenAI.
 *
 * Usage:
 *   AZURE_OPENAI_ENDPOINT=https://xxx.openai.azure.com \
 *   AZURE_OPENAI_API_KEY=xxx \
 *   AZURE_OPENAI_DEPLOYMENT=gpt-4o \
 *   pnpm start
 */

import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import {
  LlmRoundtripEngine,
  OpenAIFormatter,
  FetchTransport,
} from "@json-schema-llm/engine";

const __dirname = dirname(fileURLToPath(import.meta.url));

// ─── Configuration ──────────────────────────────────────────────────────────

const endpoint = process.env.AZURE_OPENAI_ENDPOINT;
const apiKey = process.env.AZURE_OPENAI_API_KEY;
const deployment = process.env.AZURE_OPENAI_DEPLOYMENT ?? "gpt-4o";

if (!endpoint || !apiKey) {
  console.error("Missing AZURE_OPENAI_ENDPOINT or AZURE_OPENAI_API_KEY");
  process.exit(1);
}

const apiUrl = `${endpoint}/openai/deployments/${deployment}/chat/completions?api-version=2024-08-01-preview`;

// ─── Load component schema/codec from the generated SDK output ──────────────

const schemasDir = join(__dirname, "..", "..", "dx_test_output", "ts_sdk");

// Use the "info" component (small, simple Arazzo component)
const schemaPath = join(schemasDir, "schemas", "$defs", "info", "schema.json");
const codecPath = join(schemasDir, "schemas", "$defs", "info", "codec.json");

let llmSchema: unknown;
let codecJson: string;
let schemaJson: string;

try {
  schemaJson = readFileSync(schemaPath, "utf-8");
  llmSchema = JSON.parse(schemaJson);
  codecJson = readFileSync(codecPath, "utf-8");
} catch (e) {
  // Fallback: try reading from the /tmp test output
  const tmpSchemasDir = "/tmp/ts-sdk-test/schemas";
  const tmpSchemaPath = join(tmpSchemasDir, "$defs", "info", "schema.json");
  const tmpCodecPath = join(tmpSchemasDir, "$defs", "info", "codec.json");
  schemaJson = readFileSync(tmpSchemaPath, "utf-8");
  llmSchema = JSON.parse(schemaJson);
  codecJson = readFileSync(tmpCodecPath, "utf-8");
}

// ─── WASM path resolution ──────────────────────────────────────────────────

const wasmPath =
  process.env.JSONSCHEMA_LLM_WASM_PATH ??
  join(
    __dirname,
    "..",
    "..",
    "target",
    "wasm32-wasip1",
    "release",
    "json_schema_llm_wasi.wasm"
  );

// ─── Engine roundtrip ──────────────────────────────────────────────────────

async function main() {
  console.log("=== TypeScript SDK DX Test ===\n");
  console.log(`Endpoint: ${endpoint}`);
  console.log(`Deployment: ${deployment}`);
  console.log(`WASM path: ${wasmPath}`);
  console.log(`Schema: info component`);
  console.log();

  // Step 1: Create engine
  console.log("[1/4] Creating LlmRoundtripEngine...");
  const engine = new LlmRoundtripEngine(wasmPath);

  // Step 2: Set up formatter and transport
  const formatter = new OpenAIFormatter();
  const transport = new FetchTransport();
  const config = {
    url: apiUrl,
    model: deployment,
    headers: {
      "api-key": apiKey!,
    },
  };

  // Step 3: Run generateWithPreconverted (uses SDK's pre-built schema/codec)
  console.log("[2/4] Calling Azure OpenAI with pre-converted schema...");
  const prompt =
    "Generate an Arazzo API specification info object with a creative title, summary, and version.";

  const result = await engine.generateWithPreconverted(
    schemaJson,
    codecJson,
    llmSchema,
    prompt,
    formatter,
    config,
    transport
  );

  // Step 4: Display results
  console.log("[3/4] Roundtrip complete!\n");
  console.log("--- Rehydrated Data ---");
  console.log(JSON.stringify(result.data, null, 2));
  console.log();

  console.log("[4/4] Validation Results:");
  console.log(`  isValid: ${result.isValid}`);
  console.log(`  validationErrors: ${JSON.stringify(result.validationErrors)}`);
  console.log(`  warnings: ${JSON.stringify(result.warnings)}`);
  console.log();

  if (result.isValid) {
    console.log("✅ Full roundtrip succeeded! Data is valid against the original schema.");
  } else {
    console.log("❌ Validation errors detected:");
    for (const err of result.validationErrors) {
      console.log(`  - ${err}`);
    }
  }
}

main().catch((err) => {
  console.error("Fatal error:", err);
  process.exit(1);
});
