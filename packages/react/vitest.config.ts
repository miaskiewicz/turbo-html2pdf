import { defineConfig } from "vitest/config";

export default defineConfig({
  esbuild: {
    jsx: "automatic",
  },
  test: {
    environment: "node",
    include: ["test/**/*.test.ts", "test/**/*.test.tsx"],
  },
});
