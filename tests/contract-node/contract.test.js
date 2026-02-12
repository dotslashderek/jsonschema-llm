/**
 * Node.js Contract Tests for jsonschema-llm WASM binding.
 *
 * These tests validate the WASM API contract against real fixture schemas,
 * asserting envelope shape, field types, error structures, and round-trip
 * behavior. They run via Vitest and consume the wasm-pack-built package.
 */
import { describe, it, expect, beforeAll } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { join, basename, resolve } from "node:path";

// ---------------------------------------------------------------------------
// Module-level setup
// ---------------------------------------------------------------------------

let wasm;

beforeAll(async () => {
  wasm = await import("jsonschema-llm-wasm");
});

// ---------------------------------------------------------------------------
// Fixture discovery
// ---------------------------------------------------------------------------

const FIXTURES_DIR = resolve(__dirname, "../../tests/schemas");
const TARGETS = ["openai-strict", "gemini", "claude"];

function loadFixtures() {
  return readdirSync(FIXTURES_DIR)
    .filter((f) => f.endsWith(".json"))
    .map((f) => ({
      name: basename(f, ".json"),
      schema: JSON.parse(readFileSync(join(FIXTURES_DIR, f), "utf-8")),
    }));
}

const FIXTURES = loadFixtures();

// ---------------------------------------------------------------------------
// Convert — Envelope structure
// ---------------------------------------------------------------------------

describe("convert", () => {
  describe("envelope structure", () => {
    for (const fixture of FIXTURES) {
      for (const target of TARGETS) {
        it(`${fixture.name} × ${target}: returns { apiVersion, schema, codec }`, () => {
          const result = wasm.convert(fixture.schema, { target });

          // apiVersion
          expect(result).toHaveProperty("apiVersion", "1.0");

          // schema is a plain object
          expect(result.schema).toBeDefined();
          expect(typeof result.schema).toBe("object");
          expect(result.schema).not.toBeNull();

          // codec is a plain object with expected subfields
          expect(result.codec).toBeDefined();
          expect(typeof result.codec).toBe("object");
          expect(result.codec).toHaveProperty("$schema");
          expect(result.codec).toHaveProperty("transforms");
          expect(result.codec).toHaveProperty("droppedConstraints");
          expect(Array.isArray(result.codec.transforms)).toBe(true);
          expect(Array.isArray(result.codec.droppedConstraints)).toBe(true);
        });
      }
    }
  });

  describe("schema output", () => {
    for (const fixture of FIXTURES) {
      it(`${fixture.name}: output schema has type key`, () => {
        const result = wasm.convert(fixture.schema, { target: "openai-strict" });

        // All fixture schemas should produce an object with a "type" field
        // (core normalizes to a valid JSON Schema subset)
        if (typeof result.schema === "object" && result.schema !== null) {
          expect(result.schema).toHaveProperty("type");
        }
      });
    }
  });

  describe("default options", () => {
    it("accepts undefined options", () => {
      const result = wasm.convert(FIXTURES[0].schema, undefined);
      expect(result.apiVersion).toBe("1.0");
    });

    it("accepts null options", () => {
      const result = wasm.convert(FIXTURES[0].schema, null);
      expect(result.apiVersion).toBe("1.0");
    });
  });

  describe("camelCase options", () => {
    it("accepts camelCase option keys", () => {
      const result = wasm.convert(FIXTURES[0].schema, {
        target: "openai-strict",
        maxDepth: 10,
        recursionLimit: 2,
      });
      expect(result.apiVersion).toBe("1.0");
    });
  });

  describe("kebab-case options", () => {
    it("accepts kebab-case option keys via alias", () => {
      const result = wasm.convert(FIXTURES[0].schema, {
        target: "openai-strict",
        "max-depth": 10,
        "recursion-limit": 2,
      });
      expect(result.apiVersion).toBe("1.0");
    });
  });

});

// ---------------------------------------------------------------------------
// Rehydrate — Round-trip
// ---------------------------------------------------------------------------

describe("rehydrate", () => {
  it("round-trip: convert → rehydrate preserves data", () => {
    const schema = {
      type: "object",
      properties: {
        name: { type: "string" },
        age: { type: "integer" },
      },
      required: ["name", "age"],
    };

    const { codec } = wasm.convert(schema, { target: "openai-strict" });
    const sampleData = { name: "Alice", age: 30 };

    const result = wasm.rehydrate(sampleData, codec, schema);

    expect(result).toHaveProperty("apiVersion", "1.0");
    expect(result.data).toBeDefined();
    expect(typeof result.data).toBe("object");
    expect(result.data.name).toBe("Alice");
    expect(result.data.age).toBe(30);
    expect(Array.isArray(result.warnings)).toBe(true);
  });

  const kitchenSink = FIXTURES.find((f) => f.name === "kitchen_sink");

  it.skipIf(!kitchenSink)(
    "rehydrate with dropped constraints → non-empty droppedConstraints",
    () => {

    const { codec } = wasm.convert(kitchenSink.schema, {
      target: "openai-strict",
    });

    // kitchen_sink should have dropped constraints (e.g. minLength, pattern, etc.)
    expect(
      codec.droppedConstraints.length,
      "kitchen_sink should have dropped constraints for openai-strict",
    ).toBeGreaterThan(0);

    // Rehydrate with data — should succeed and produce warnings array
    const result = wasm.rehydrate({}, codec, kitchenSink.schema);
    expect(result).toHaveProperty("apiVersion", "1.0");
    expect(Array.isArray(result.warnings)).toBe(true);
    },
  );
});

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

describe("error handling", () => {
  it("bad options type → structured error with code", () => {
    try {
      wasm.convert(FIXTURES[0].schema, 42);
      expect.unreachable("should have thrown");
    } catch (e) {
      expect(e).toHaveProperty("code", "json_parse_error");
      expect(e).toHaveProperty("message");
      expect(typeof e.message).toBe("string");
    }
  });

  it("bad codec type → structured error with code", () => {
    try {
      wasm.rehydrate({ name: "test" }, 42, {});
      expect.unreachable("should have thrown");
    } catch (e) {
      expect(e).toHaveProperty("code", "json_parse_error");
      expect(e).toHaveProperty("message");
      expect(typeof e.message).toBe("string");
    }
  });

  it("bad codec version → structured error", () => {
    const badCodec = {
      $schema: "https://jsonschema-llm.dev/codec/v999",
      transforms: [],
      droppedConstraints: [],
    };
    try {
      wasm.rehydrate({}, badCodec, {});
      expect.unreachable("should have thrown");
    } catch (e) {
      // Should be a structured error from core
      expect(e).toHaveProperty("code");
      expect(e).toHaveProperty("message");
    }
  });
});
