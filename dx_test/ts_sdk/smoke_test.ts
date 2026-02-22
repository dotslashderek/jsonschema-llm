/**
 * Smoke test — verify SDK imports resolve and WASI engine initializes.
 * No API credentials needed.
 */

import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));

// Test 1: Import the engine package
console.log("[1/4] Testing engine import...");
let LlmRoundtripEngine: any, OpenAIFormatter: any, FetchTransport: any;
try {
  const engine = await import("@json-schema-llm/engine");
  LlmRoundtripEngine = engine.LlmRoundtripEngine;
  OpenAIFormatter = engine.OpenAIFormatter;
  FetchTransport = engine.FetchTransport;
  console.log("  ✅ Engine package imported successfully");
  console.log(`  Exports: ${Object.keys(engine).join(", ")}`);
} catch (e) {
  console.error("  ❌ Engine import failed:", e);
  process.exit(1);
}

// Test 2: Load schema/codec from generated SDK output
console.log("\n[2/4] Testing schema/codec loading from generated SDK...");
const schemaPath = "/tmp/ts-sdk-test/schemas/$defs/info/schema.json";
const codecPath = "/tmp/ts-sdk-test/schemas/$defs/info/codec.json";
try {
  const schemaJson = readFileSync(schemaPath, "utf-8");
  const codecJson = readFileSync(codecPath, "utf-8");
  const llmSchema = JSON.parse(schemaJson);
  const codec = JSON.parse(codecJson);
  console.log("  ✅ Schema loaded:", Object.keys(llmSchema).join(", "));
  console.log("  ✅ Codec loaded:", Object.keys(codec).join(", "));
} catch (e) {
  console.error("  ❌ Schema/codec loading failed:", e);
  process.exit(1);
}

// Test 3: Initialize WASI engine
console.log("\n[3/4] Testing WASI engine initialization...");
const wasmPath = join(
  __dirname,
  "..",
  "..",
  "target",
  "wasm32-wasip1",
  "release",
  "json_schema_llm_wasi.wasm"
);
try {
  const engine = new LlmRoundtripEngine(wasmPath);
  console.log("  ✅ LlmRoundtripEngine created successfully");
} catch (e) {
  console.error("  ❌ Engine initialization failed:", e);
  process.exit(1);
}

// Test 4: Verify formatter and transport instantiation
console.log("\n[4/4] Testing formatter and transport...");
try {
  const formatter = new OpenAIFormatter();
  const transport = new FetchTransport();

  // Test format() produces a valid request
  const testSchema = { type: "object", properties: { name: { type: "string" } } };
  const request = formatter.format("test prompt", testSchema, {
    url: "https://example.com/api",
    model: "gpt-4o",
    headers: { "api-key": "test" },
  });

  console.log("  ✅ OpenAIFormatter.format() works");
  console.log(`  Request URL: ${request.url}`);
  console.log(`  Request body length: ${request.body.length} chars`);

  // Verify body structure
  const body = JSON.parse(request.body);
  console.log(`  Models: ${body.model}`);
  console.log(`  response_format.type: ${body.response_format?.type}`);
  console.log(`  strict: ${body.response_format?.json_schema?.strict}`);

  console.log("  ✅ FetchTransport instantiated");
} catch (e) {
  console.error("  ❌ Formatter/transport test failed:", e);
  process.exit(1);
}

console.log("\n=== All smoke tests passed! ===");
console.log("\nTo run the full e2e roundtrip with Azure OpenAI, set:");
console.log("  AZURE_OPENAI_ENDPOINT=<your-endpoint>");
console.log("  AZURE_OPENAI_API_KEY=<your-key>");
console.log("  AZURE_OPENAI_DEPLOYMENT=<your-deployment>");
console.log("Then run: pnpm start");
