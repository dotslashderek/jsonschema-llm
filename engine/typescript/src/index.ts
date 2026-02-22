/**
 * @json-schema-llm/engine — barrel exports.
 *
 * Public API surface for the LLM Roundtrip Engine.
 */

// Core engine
export { LlmRoundtripEngine } from "./engine.js";

// Types
export type { ProviderConfig, LlmRequest, RoundtripResult } from "./types.js";

// Formatter interface + built-in formatters
export type { ProviderFormatter } from "./formatter.js";
export { OpenAIFormatter } from "./formatters/openai.js";

// Transport interface + built-in transports
export type { LlmTransport } from "./transport.js";
export { FetchTransport, LlmTransportError } from "./transport.js";

// Errors
export {
  SchemaConversionError,
  RehydrationError,
  ResponseParsingError,
} from "./errors.js";
