/**
 * LLM Roundtrip Engine — orchestrates convert → format → call → rehydrate → validate.
 *
 * Mirrors Python engine's engine.py architecture. Wraps the @json-schema-llm/wasi
 * Engine bindings and adds the formatter/transport orchestration layer.
 */

import { Engine as WasiEngine, type ConvertResult } from "@json-schema-llm/wasi";
import {
  SchemaConversionError,
  RehydrationError,
  ResponseParsingError,
} from "./errors.js";
import type { ProviderFormatter } from "./formatter.js";
import type { LlmTransport } from "./transport.js";
import type { ProviderConfig, RoundtripResult } from "./types.js";

/**
 * LlmRoundtripEngine orchestrates the full LLM roundtrip.
 *
 * Lifecycle: convert schema → format request → call LLM → extract content →
 * rehydrate output → validate against original schema.
 *
 * The WASI Engine and compiled Module are cached at init time.
 * A fresh WASI instance is created per call.
 *
 * @example
 * ```ts
 * import { LlmRoundtripEngine, OpenAIFormatter, FetchTransport } from "@json-schema-llm/engine";
 *
 * const engine = new LlmRoundtripEngine();
 * const result = await engine.generate(
 *   schemaJson,
 *   "Generate a user profile",
 *   new OpenAIFormatter(),
 *   { url: "https://api.openai.com/v1/chat/completions", model: "gpt-4o", headers: { Authorization: `Bearer ${key}` } },
 *   new FetchTransport(),
 * );
 * console.log(result.data);
 * ```
 */
export class LlmRoundtripEngine {
  private readonly wasi: WasiEngine;

  /**
   * @param wasmPath - Optional explicit path to the WASI binary. Falls back to
   *                   JSL_WASM_PATH env var → import.meta.resolve → repo-relative.
   */
  constructor(wasmPath?: string) {
    this.wasi = new WasiEngine(wasmPath);
  }

  /**
   * Full roundtrip: convert → format → call LLM → rehydrate → validate.
   *
   * @param schemaJson - The original JSON Schema as a string.
   * @param prompt - The user's natural language prompt.
   * @param formatter - Provider-specific request formatter.
   * @param config - Provider endpoint configuration.
   * @param transport - Consumer-provided HTTP transport.
   * @returns RoundtripResult with rehydrated data and validation status.
   */
  async generate(
    schemaJson: string,
    prompt: string,
    formatter: ProviderFormatter,
    config: ProviderConfig,
    transport: LlmTransport,
  ): Promise<RoundtripResult> {
    // Step 1: Convert schema to LLM-compatible form
    let convertResult: ConvertResult;
    try {
      const schema = JSON.parse(schemaJson);
      convertResult = await this.wasi.convert(schema);
    } catch (e) {
      if (e instanceof SyntaxError) {
        throw new SchemaConversionError(`Invalid JSON schema: ${e.message}`);
      }
      throw new SchemaConversionError(`Schema conversion failed: ${e}`);
    }

    const llmSchema = convertResult.schema;
    const codec = convertResult.codec;

    return this.generateWithPreconverted(
      schemaJson,
      JSON.stringify(codec),
      llmSchema,
      prompt,
      formatter,
      config,
      transport,
    );
  }

  /**
   * Roundtrip with pre-converted schema (skips the convert step).
   *
   * Use when you have pre-built schema/codec from gen-sdk.
   *
   * @param schemaJson - The original JSON Schema as a string.
   * @param codecJson - The codec (rehydration map) as a string.
   * @param llmSchema - The LLM-compatible schema (already converted).
   * @param prompt - The user's natural language prompt.
   * @param formatter - Provider-specific request formatter.
   * @param config - Provider endpoint configuration.
   * @param transport - Consumer-provided HTTP transport.
   * @returns RoundtripResult with rehydrated data and validation status.
   */
  async generateWithPreconverted(
    schemaJson: string,
    codecJson: string,
    llmSchema: unknown,
    prompt: string,
    formatter: ProviderFormatter,
    config: ProviderConfig,
    transport: LlmTransport,
  ): Promise<RoundtripResult> {
    // Step 2: Format the request for the provider
    const request = formatter.format(prompt, llmSchema, config);

    // Step 3: Call the LLM via consumer transport
    const rawResponse = await transport.execute(request);

    // Step 4: Extract content from the response
    let content: string;
    try {
      content = formatter.extractContent(rawResponse);
    } catch (e) {
      if (e instanceof ResponseParsingError) throw e;
      throw new ResponseParsingError(`Failed to extract content: ${e}`);
    }

    // Step 5: Rehydrate the output
    let rehydratedData: unknown;
    let warnings: string[] = [];
    try {
      const schema = JSON.parse(schemaJson);
      const rehydrateResult = await this.wasi.rehydrate(
        JSON.parse(content),
        JSON.parse(codecJson),
        schema,
      );
      rehydratedData = rehydrateResult.data;
      warnings = (rehydrateResult.warnings ?? []).map(
        (w) => `[${w.kind.type}] ${w.message} at ${w.dataPath}`,
      );
    } catch (e) {
      if (e instanceof RehydrationError) throw e;
      throw new RehydrationError(`Rehydration failed: ${e}`);
    }

    // Step 6: Validate against original schema
    // TODO: Integrate ajv for JSON Schema validation to achieve feature parity
    // with the Python engine (which uses the `jsonschema` package) and the Java
    // engine (which uses `json-schema-validator`). Until then, validationErrors
    // will always be empty and isValid will always be true.
    const validationErrors: string[] = [];

    return {
      data: rehydratedData,
      rawLlmResponse: JSON.parse(rawResponse),
      warnings,
      validationErrors,
      isValid: validationErrors.length === 0,
    };
  }
}
