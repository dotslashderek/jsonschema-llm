import { describe, it, expect, beforeAll, afterAll } from "vitest";
import { readFileSync } from "fs";
import { resolve } from "path";
import { Engine, JslError } from "../src/index.js";

interface Fixture {
  id: string;
  description: string;
  input: {
    schema?: unknown;
    schema_raw?: string;
    options?: Record<string, unknown>;
    data?: unknown;
    codec_raw?: string;
  };
  expected: Record<string, unknown>;
}

interface FixtureFile {
  suites: Record<string, { description: string; fixtures: Fixture[] }>;
}

const fixturesPath = resolve(__dirname, "../../../tests/conformance/fixtures.json");
const fixtures: FixtureFile = JSON.parse(readFileSync(fixturesPath, "utf-8"));

describe("Conformance: convert", () => {
  let engine: Engine;

  beforeAll(() => {
    engine = new Engine();
  });

  afterAll(() => {
    engine.close();
  });

  for (const fx of fixtures.suites.convert.fixtures) {
    it(fx.id, async () => {
      const { input, expected } = fx;

      // Error case: schema_raw → raw FFI
      if (input.schema_raw !== undefined) {
        expect(expected.is_error).toBe(true);
        const optsJson = JSON.stringify(input.options ?? {});

        try {
          await (engine as any).callJsl(
            "jsl_convert",
            input.schema_raw,
            optsJson
          );
          expect.unreachable("should have thrown");
        } catch (err) {
          expect(err).toBeInstanceOf(JslError);
          const jslErr = err as JslError;

          if (expected.error_has_keys) {
            for (const key of expected.error_has_keys as string[]) {
              expect(jslErr).toHaveProperty(key);
            }
          }
          if (expected.error_code) {
            expect(jslErr.code).toBe(expected.error_code);
          }
        }
        return;
      }

      // Normal convert
      const result = await engine.convert(
        input.schema,
        input.options as any
      );

      if (expected.has_keys) {
        for (const key of expected.has_keys as string[]) {
          expect(result).toHaveProperty(key);
        }
      }

      if (expected.apiVersion !== undefined) {
        expect(result.apiVersion).toBe(expected.apiVersion);
      }

      if (expected.schema_has_properties) {
        expect(result.schema).toHaveProperty("properties");
      }

      if (expected.codec_has_schema_uri) {
        expect(result.codec).toBeTruthy();
      }
    });
  }
});

describe("Conformance: roundtrip", () => {
  let engine: Engine;

  beforeAll(() => {
    engine = new Engine();
  });

  afterAll(() => {
    engine.close();
  });

  for (const fx of fixtures.suites.roundtrip.fixtures) {
    it(fx.id, async () => {
      const { input, expected } = fx;

      const convertResult = await engine.convert(
        input.schema,
        input.options as any
      );

      const rehydrateResult = await engine.rehydrate(
        input.data,
        convertResult.codec,
        input.schema
      );

      if (expected.has_keys) {
        for (const key of expected.has_keys as string[]) {
          expect(rehydrateResult).toHaveProperty(key);
        }
      }

      if (expected.apiVersion !== undefined) {
        expect(rehydrateResult.apiVersion).toBe(expected.apiVersion);
      }

      if (expected.data !== undefined) {
        expect(rehydrateResult.data).toEqual(expected.data);
      }

      if (expected.data_user_name !== undefined) {
        expect((rehydrateResult.data as any).user.name).toBe(
          expected.data_user_name
        );
      }

      if (expected.data_value !== undefined) {
        expect((rehydrateResult.data as any).value).toBe(expected.data_value);
      }

      if (expected.warnings_is_array) {
        expect(Array.isArray(rehydrateResult.warnings)).toBe(true);
      }
    });
  }
});

describe("Conformance: rehydrate_error", () => {
  let engine: Engine;

  beforeAll(() => {
    engine = new Engine();
  });

  afterAll(() => {
    engine.close();
  });

  for (const fx of fixtures.suites.rehydrate_error.fixtures) {
    it(fx.id, async () => {
      const { input, expected } = fx;

      expect(expected.is_error).toBe(true);

      const dataJson = JSON.stringify(input.data);
      const schemaJson = JSON.stringify(input.schema);
      const codecArg = input.codec_raw ?? "{}";

      try {
        await (engine as any).callJsl(
          "jsl_rehydrate",
          dataJson,
          codecArg,
          schemaJson
        );
        expect.unreachable("should have thrown");
      } catch (err) {
        expect(err).toBeInstanceOf(JslError);
        const jslErr = err as JslError;

        if (expected.error_has_keys) {
          for (const key of expected.error_has_keys as string[]) {
            expect(jslErr).toHaveProperty(key);
          }
        }
      }
    });
  }
});
