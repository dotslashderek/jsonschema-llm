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
    pointer?: string;
    convert_options?: Record<string, unknown>;
    extract_options?: Record<string, unknown>;
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

describe("Conformance: list_components", () => {
  let engine: Engine;

  beforeAll(() => {
    engine = new Engine();
  });

  afterAll(() => {
    engine.close();
  });

  for (const fx of fixtures.suites.list_components.fixtures) {
    it(fx.id, async () => {
      const { input, expected } = fx;

      // Error case
      if (input.schema_raw !== undefined) {
        expect(expected.is_error).toBe(true);
        try {
          await (engine as any).callJsl("jsl_list_components", input.schema_raw);
          expect.unreachable("should have thrown");
        } catch (err) {
          expect(err).toBeInstanceOf(JslError);
          if (expected.error_has_keys) {
            for (const key of expected.error_has_keys as string[]) {
              expect(err).toHaveProperty(key);
            }
          }
        }
        return;
      }

      const result = await engine.listComponents(input.schema);

      if (expected.apiVersion !== undefined) {
        expect(result.apiVersion).toBe(expected.apiVersion);
      }
      if (expected.components !== undefined) {
        expect(result.components).toEqual(expected.components);
      }
    });
  }
});

describe("Conformance: extract_component", () => {
  let engine: Engine;

  beforeAll(() => {
    engine = new Engine();
  });

  afterAll(() => {
    engine.close();
  });

  for (const fx of fixtures.suites.extract_component.fixtures) {
    it(fx.id, async () => {
      const { input, expected } = fx;

      // Error case
      if (expected.is_error) {
        try {
          if (input.schema_raw !== undefined) {
            await (engine as any).callJsl(
              "jsl_extract_component",
              input.schema_raw,
              input.pointer,
              "{}"
            );
          } else {
            await engine.extractComponent(input.schema, input.pointer!);
          }
          expect.unreachable("should have thrown");
        } catch (err) {
          expect(err).toBeInstanceOf(JslError);
          if (expected.error_has_keys) {
            for (const key of expected.error_has_keys as string[]) {
              expect(err).toHaveProperty(key);
            }
          }
        }
        return;
      }

      const result = await engine.extractComponent(input.schema, input.pointer!);

      if (expected.apiVersion !== undefined) {
        expect(result.apiVersion).toBe(expected.apiVersion);
      }
      if (expected.pointer !== undefined) {
        expect(result.pointer).toBe(expected.pointer);
      }
      if (expected.schema_is_object) {
        expect(typeof result.schema).toBe("object");
      }
      if (expected.dependency_count !== undefined) {
        expect(result.dependencyCount).toBe(expected.dependency_count);
      }
      if (expected.dependency_count_gte !== undefined) {
        expect(result.dependencyCount).toBeGreaterThanOrEqual(
          expected.dependency_count_gte as number
        );
      }
    });
  }
});

describe("Conformance: convert_all_components", () => {
  let engine: Engine;

  beforeAll(() => {
    engine = new Engine();
  });

  afterAll(() => {
    engine.close();
  });

  for (const fx of fixtures.suites.convert_all_components.fixtures) {
    it(fx.id, async () => {
      const { input, expected } = fx;

      // Error case
      if (input.schema_raw !== undefined) {
        expect(expected.is_error).toBe(true);
        const convOpts = JSON.stringify(input.convert_options ?? {});
        const extOpts = JSON.stringify(input.extract_options ?? {});
        try {
          await (engine as any).callJsl(
            "jsl_convert_all_components",
            input.schema_raw,
            convOpts,
            extOpts
          );
          expect.unreachable("should have thrown");
        } catch (err) {
          expect(err).toBeInstanceOf(JslError);
          if (expected.error_has_keys) {
            for (const key of expected.error_has_keys as string[]) {
              expect(err).toHaveProperty(key);
            }
          }
        }
        return;
      }

      const result = await engine.convertAllComponents(input.schema);

      if (expected.apiVersion !== undefined) {
        expect(result.apiVersion).toBe(expected.apiVersion);
      }
      if (expected.full_is_object) {
        expect(typeof result.full).toBe("object");
      }
      if (expected.components_count !== undefined) {
        expect(result.components.length).toBe(expected.components_count);
      }
    });
  }
});
