import { defineConfig } from "vitest/config";

// Pure-logic tests only (formatting, row model, filters, i18n JSON parity) —
// no DOM needed, keep the node environment for speed.
export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
