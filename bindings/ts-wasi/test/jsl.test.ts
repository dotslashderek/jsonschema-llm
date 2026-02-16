import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { Engine, JslError } from "../src/index.js";

describe("JsonSchemaLlm WASI", () => {
  let engine: Engine;

  beforeAll(() => {
    engine = new Engine();
  });

  afterAll(() => {
    engine.close();
  });

  it("converts a simple schema", async () => {
    const schema = {
      type: "object",
      properties: {
        name: { type: "string" },
        age: { type: "integer", minimum: 0 },
      },
      required: ["name", "age"],
    };

    const result = await engine.convert(schema);
    expect(result.apiVersion).toBeTruthy();
    expect(result.schema).toBeTruthy();
    expect(result.codec).toBeTruthy();
  });

  it("returns error for invalid JSON", async () => {
    await expect(
      (engine as any).callJsl("jsl_convert", "NOT VALID JSON", "{}")
    ).rejects.toThrow(JslError);
  });

  it("roundtrips convert + rehydrate", async () => {
    const schema = {
      type: "object",
      properties: {
        name: { type: "string" },
        age: { type: "integer", minimum: 0 },
      },
      required: ["name", "age"],
    };

    const convertResult = await engine.convert(schema);
    const data = { name: "Ada", age: 36 };
    const rehydrated = await engine.rehydrate(
      data,
      convertResult.codec,
      schema
    );

    expect(rehydrated.apiVersion).toBeTruthy();
    expect((rehydrated.data as any).name).toBe("Ada");
  });

  it("returns error for invalid codec", async () => {
    await expect(
      (engine as any).callJsl(
        "jsl_rehydrate",
        '{"key":"value"}',
        "NOT VALID JSON",
        '{"type":"object"}'
      )
    ).rejects.toThrow(JslError);
  });

  it("handles multiple sequential calls", async () => {
    const schema = {
      type: "object",
      properties: { x: { type: "number" } },
    };
    for (let i = 0; i < 5; i++) {
      const result = await engine.convert(schema);
      expect(result.schema).toBeTruthy();
    }
  });
});
