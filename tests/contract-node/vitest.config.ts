import { configDefaults, defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    globals: true,
    testTimeout: 30_000,
    exclude: [...configDefaults.exclude, "types.test.ts"],
  },
});
