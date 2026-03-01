/**
 * SchemaLlmEngine — High-level facade for json-schema-llm WASI operations.
 *
 * Provides the consumer-friendly API described in Epic #165:
 * - Async factory with WASM auto-discovery cascade
 * - Typed results for all operations
 * - Resource lifecycle management
 *
 * @example
 * ```ts
 * import { SchemaLlmEngine } from '@json-schema-llm/wasi';
 *
 * const engine = await SchemaLlmEngine.create();
 * const { schema, codec } = await engine.convert(mySchema, { target: 'openai-strict' });
 * const { data } = await engine.rehydrate(llmOutput, codec, mySchema);
 * engine.close();
 * ```
 */

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { Engine } from "./core.js";
import type {
  ConvertOptions,
  ConvertResult,
  RehydrateResult,
  ExtractOptions,
  ExtractResult,
  ListComponentsResult,
  ConvertAllResult,
} from "./index.js";

/** Options for creating a SchemaLlmEngine instance. */
export interface SchemaLlmEngineOptions {
  /** Explicit path to the WASI binary. Overrides all auto-discovery. */
  readonly wasmPath?: string;
}

/**
 * Resolve the WASM binary path using a cascade:
 * 1. Explicit path (from options)
 * 2. JSL_WASM_PATH environment variable
 * 3. Package-bundled WASM (npm install scenario)
 * 4. Repo-relative fallback (dev/CI)
 */
function resolveWasmPath(options?: SchemaLlmEngineOptions): string {
  // Tier 1: Explicit path
  if (options?.wasmPath) {
    if (!existsSync(options.wasmPath)) {
      throw new Error(
        `WASM binary not found at explicit path: ${options.wasmPath}`
      );
    }
    return options.wasmPath;
  }

  // Tier 2: Environment variable
  const envPath = process.env.JSL_WASM_PATH;
  if (envPath) {
    if (!existsSync(envPath)) {
      throw new Error(`WASM binary not found at JSL_WASM_PATH: ${envPath}`);
    }
    return envPath;
  }

  // Tier 3: Package-bundled WASM (npm install scenario)
  // Uses import.meta.dirname for ESM compatibility (Node 20.11+)
  const packageWasm = join(
    import.meta.dirname,
    "..",
    "wasm",
    "json_schema_llm_wasi.wasm"
  );
  if (existsSync(packageWasm)) {
    return packageWasm;
  }

  // Tier 4: Repo-relative fallback (dev/CI)
  const repoWasm = join(
    import.meta.dirname,
    "..",
    "..",
    "..",
    "target",
    "wasm32-wasip1",
    "release",
    "json_schema_llm_wasi.wasm"
  );
  if (existsSync(repoWasm)) {
    return repoWasm;
  }

  throw new Error(
    "WASM binary not found. Set JSL_WASM_PATH, " +
      "place the binary in @json-schema-llm/wasi/wasm/, " +
      "or build the WASI target (make build-wasi)."
  );
}

export class SchemaLlmEngine {
  private readonly engine: Engine;

  private constructor(engine: Engine) {
    this.engine = engine;
  }

  /**
   * Create a new SchemaLlmEngine with WASM auto-discovery.
   *
   * Resolution cascade:
   * 1. Explicit `wasmPath` option
   * 2. `JSL_WASM_PATH` env var
   * 3. Package-bundled WASM (`wasm/json_schema_llm_wasi.wasm`)
   * 4. Repo-relative fallback (`target/wasm32-wasip1/release/`)
   */
  static async create(options?: SchemaLlmEngineOptions): Promise<SchemaLlmEngine> {
    const wasmPath = resolveWasmPath(options);
    const engine = new Engine(wasmPath);
    return new SchemaLlmEngine(engine);
  }

  /** Convert a JSON Schema to LLM-compatible form. */
  async convert(schema: unknown, options?: ConvertOptions): Promise<ConvertResult> {
    return this.engine.convert(schema, options);
  }

  /**
   * Apply RFC 6902 JSON Patch operations to a JSON Schema via the WASM core.
   *
   * @param schema    - The JSON Schema to patch.
   * @param patchJson - RFC 6902 patch operations as a JSON string.
   * @returns The patched schema as a plain object.
   */
  async applyPatch(
    schema: unknown,
    patchJson: string,
  ): Promise<Record<string, unknown>> {
    return this.engine.applyPatch(schema, patchJson);
  }

  /** Rehydrate LLM output back to the original schema shape. */
  async rehydrate(
    data: unknown,
    codec: unknown,
    schema: unknown
  ): Promise<RehydrateResult> {
    return this.engine.rehydrate(data, codec, schema);
  }

  /** List extractable components from a schema. */
  async listComponents(schema: unknown): Promise<ListComponentsResult> {
    return this.engine.listComponents(schema);
  }

  /** Extract a single component from a schema by JSON pointer. */
  async extractComponent(
    schema: unknown,
    pointer: string,
    options?: ExtractOptions
  ): Promise<ExtractResult> {
    return this.engine.extractComponent(schema, pointer, options);
  }

  /** Convert all components in a schema at once. */
  async convertAllComponents(
    schema: unknown,
    convertOpts?: ConvertOptions,
    extractOpts?: ExtractOptions
  ): Promise<ConvertAllResult> {
    return this.engine.convertAllComponents(schema, convertOpts, extractOpts);
  }

  /** Release cached WASM module and free resources. */
  close(): void {
    this.engine.close();
  }
}
