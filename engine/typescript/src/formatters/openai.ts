/**
 * OpenAI Chat Completions formatter — formats requests for OpenAI's
 * structured output API using response_format.
 *
 * Mirrors Python engine's formatters/chat_completions.py.
 */

import { ResponseParsingError } from "../errors.js";
import type { ProviderFormatter } from "../formatter.js";
import type { LlmRequest, ProviderConfig } from "../types.js";

/** Formatter for OpenAI Chat Completions API with structured output. */
export class OpenAIFormatter implements ProviderFormatter {
  format(prompt: string, llmSchema: unknown, config: ProviderConfig): LlmRequest {
    const body = {
      model: config.model,
      messages: [
        {
          role: "user" as const,
          content: prompt,
        },
      ],
      response_format: {
        type: "json_schema",
        json_schema: {
          name: "response",
          strict: true,
          schema: llmSchema,
        },
      },
    };

    return {
      url: config.url,
      headers: {
        "Content-Type": "application/json",
        ...(config.headers ?? {}),
      },
      body: JSON.stringify(body),
    };
  }

  extractContent(rawResponse: string): string {
    try {
      const parsed = JSON.parse(rawResponse);
      const content = parsed?.choices?.[0]?.message?.content;
      if (typeof content !== "string") {
        throw new ResponseParsingError(
          "No content in response: choices[0].message.content is missing or not a string",
        );
      }
      return content;
    } catch (e) {
      if (e instanceof ResponseParsingError) throw e;
      throw new ResponseParsingError(`Failed to parse OpenAI response: ${e}`);
    }
  }
}
