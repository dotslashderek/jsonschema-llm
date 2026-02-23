/**
 * Engine unit tests — validates LlmRoundtripEngine lifecycle and schema ops.
 *
 * Uses a mock transport to avoid real HTTP calls while testing
 * the convert → format → rehydrate → validate pipeline.
 */

import { describe, it, expect, afterEach } from "vitest";
import { LlmRoundtripEngine } from "../engine.js";
import type { LlmTransport } from "../transport.js";
import type { ProviderFormatter } from "../formatter.js";
import type { LlmRequest, ProviderConfig } from "../types.js";

const SIMPLE_SCHEMA = JSON.stringify({
  type: "object",
  properties: {
    name: { type: "string" },
    age: { type: "integer", minimum: 0 },
  },
  required: ["name", "age"],
});

/** Mock transport that returns a canned LLM response. */
class MockTransport implements LlmTransport {
  constructor(private readonly responseData: unknown) {}

  async execute(_request: LlmRequest): Promise<string> {
    return JSON.stringify({
      choices: [
        {
          message: {
            content: JSON.stringify(this.responseData),
          },
        },
      ],
    });
  }
}

/** Mock formatter that uses OpenAI-style request/response format. */
class MockFormatter implements ProviderFormatter {
  format(prompt: string, llmSchema: unknown, config: ProviderConfig): LlmRequest {
    return {
      url: config.url,
      headers: { "Content-Type": "application/json", ...config.headers },
      body: JSON.stringify({
        model: config.model,
        messages: [{ role: "user", content: prompt }],
        response_format: { type: "json_schema", json_schema: { name: "response", schema: llmSchema } },
      }),
    };
  }

  extractContent(rawResponse: string): string {
    const parsed = JSON.parse(rawResponse);
    return parsed.choices[0].message.content;
  }
}

describe("LlmRoundtripEngine", () => {
  let engine: LlmRoundtripEngine;

  afterEach(() => {
    engine?.close();
  });

  it("constructs without error", () => {
    engine = new LlmRoundtripEngine();
    expect(engine).toBeInstanceOf(LlmRoundtripEngine);
  });

  describe("generate()", () => {
    it("performs full roundtrip with mock transport", async () => {
      engine = new LlmRoundtripEngine();
      const mockData = { name: "Ada", age: 36 };
      const transport = new MockTransport(mockData);
      const formatter = new MockFormatter();
      const config: ProviderConfig = {
        url: "https://mock.api/v1/chat/completions",
        model: "mock-model",
      };

      const result = await engine.generate(
        SIMPLE_SCHEMA,
        "Generate a user profile",
        formatter,
        config,
        transport,
      );

      expect(result.data).toEqual(mockData);
      expect(result.isValid).toBe(true);
      expect(result.validationErrors).toHaveLength(0);
      expect(result.rawLlmResponse).toBeTruthy();
    });

    it("reports validation errors for invalid data", async () => {
      engine = new LlmRoundtripEngine();
      // Missing required 'name' field
      const mockData = { age: 36 };
      const transport = new MockTransport(mockData);
      const formatter = new MockFormatter();
      const config: ProviderConfig = {
        url: "https://mock.api/v1/chat/completions",
        model: "mock-model",
      };

      const result = await engine.generate(
        SIMPLE_SCHEMA,
        "Generate a user profile",
        formatter,
        config,
        transport,
      );

      expect(result.isValid).toBe(false);
      expect(result.validationErrors.length).toBeGreaterThan(0);
    });
  });

  describe("generateWithPreconverted()", () => {
    it("works with pre-converted schema and codec", async () => {
      engine = new LlmRoundtripEngine();
      const mockData = { name: "Charles", age: 42 };
      const transport = new MockTransport(mockData);
      const formatter = new MockFormatter();
      const config: ProviderConfig = {
        url: "https://mock.api/v1/chat/completions",
        model: "mock-model",
      };

      // Pre-convert using the underlying binding
      const { Engine } = await import("@json-schema-llm/wasi");
      const wasiEngine = new Engine();
      const converted = await wasiEngine.convert(JSON.parse(SIMPLE_SCHEMA));
      wasiEngine.close();

      const result = await engine.generateWithPreconverted(
        SIMPLE_SCHEMA,
        JSON.stringify(converted.codec),
        converted.schema,
        "Generate a user",
        formatter,
        config,
        transport,
      );

      expect(result.data).toEqual(mockData);
      expect(result.isValid).toBe(true);
    });
  });

  describe("close()", () => {
    it("can be called safely", () => {
      engine = new LlmRoundtripEngine();
      engine.close();
      // Should not throw
    });
  });
});
