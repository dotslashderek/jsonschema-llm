/**
 * Transport abstraction for LLM HTTP calls.
 *
 * Mirrors Python engine's transport.py but async (TS is naturally async).
 * Includes a default FetchTransport using Node 18+ native fetch.
 */

import type { LlmRequest } from "./types.js";

/** Transport interface — consumer-provided or use the default FetchTransport. */
export interface LlmTransport {
  /**
   * Execute an LLM request and return the raw response body.
   *
   * @param request - The formatted LLM request.
   * @returns The raw HTTP response body as a string.
   * @throws LlmTransportError on HTTP errors.
   */
  execute(request: LlmRequest): Promise<string>;
}

/** Error thrown by transport implementations on HTTP failures. */
export class LlmTransportError extends Error {
  readonly statusCode: number;

  constructor(message: string, statusCode: number) {
    super(message);
    this.name = "LlmTransportError";
    this.statusCode = statusCode;
  }

  get isHttpError(): boolean {
    return this.statusCode >= 400;
  }
}

/**
 * Default transport using Node 18+ native fetch.
 *
 * Zero-dependency convenience implementation. Consumers can replace
 * with their own transport (e.g., axios, got, custom retry logic).
 */
export class FetchTransport implements LlmTransport {
  async execute(request: LlmRequest): Promise<string> {
    const response = await fetch(request.url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...request.headers,
      },
      body: request.body,
    });

    if (!response.ok) {
      const body = await response.text().catch(() => "<unreadable>");
      throw new LlmTransportError(
        `HTTP ${response.status}: ${body}`,
        response.status,
      );
    }

    return response.text();
  }
}
