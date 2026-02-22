/**
 * Core types for the json-schema-llm TypeScript engine.
 *
 * Mirrors the Python engine's types.py — ProviderConfig, LlmRequest, RoundtripResult.
 */

/** Configuration for an LLM provider endpoint. */
export interface ProviderConfig {
  /** The provider API endpoint URL. */
  readonly url: string;
  /** The model identifier (e.g., "gpt-4o", "gemini-pro"). */
  readonly model: string;
  /** Additional HTTP headers (e.g., Authorization). */
  readonly headers?: Record<string, string>;
}

/** Immutable request to send to an LLM provider. */
export interface LlmRequest {
  /** The provider endpoint URL. */
  readonly url: string;
  /** HTTP headers (e.g., Authorization, Content-Type). */
  readonly headers: Record<string, string>;
  /** The serialized request body (JSON string). */
  readonly body: string;
}

/** Result of a full LLM roundtrip: convert → call LLM → rehydrate → validate. */
export interface RoundtripResult {
  /** The rehydrated, validated output matching the original schema shape. */
  readonly data: unknown;
  /** The raw response from the LLM provider (for debugging/audit). */
  readonly rawLlmResponse: unknown;
  /** Advisory warnings from the rehydration step. */
  readonly warnings: readonly string[];
  /** JSON Schema validation errors against the original schema. */
  readonly validationErrors: readonly string[];
  /** True if the rehydrated data passes JSON Schema validation. */
  readonly isValid: boolean;
}
