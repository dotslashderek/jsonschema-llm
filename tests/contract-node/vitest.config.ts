import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    globals: true,
    testTimeout: 30_000,
    exclude: ["types.test.ts", "node_modules/**"],
  },
});
