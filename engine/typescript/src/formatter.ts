/**
 * Strategy interface for formatting LLM requests per provider.
 *
 * Each provider has its own request/response JSON shape.
 * Implementations handle the formatting and content extraction.
 *
 * Mirrors Python engine's formatter.py ProviderFormatter protocol.
 */

import type { LlmRequest, ProviderConfig } from "./types.js";

/** Strategy interface for formatting LLM requests per provider. */
export interface ProviderFormatter {
  /**
   * Format a prompt + schema into a provider-specific request.
   *
   * @param prompt - The user's natural language prompt.
   * @param llmSchema - The LLM-compatible schema (already converted).
   * @param config - Provider endpoint configuration.
   * @returns An LlmRequest ready to send.
   */
  format(prompt: string, llmSchema: unknown, config: ProviderConfig): LlmRequest;

  /**
   * Extract the structured content from a raw LLM response.
   *
   * @param rawResponse - The raw HTTP response body as a string.
   * @returns The extracted content as a JSON string.
   */
  extractContent(rawResponse: string): string;
}
