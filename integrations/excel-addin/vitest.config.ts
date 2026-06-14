import { defineConfig } from "vitest/config";

// Vitest runs the pure `src/lib` modules (and their tests) under Node. The
// source uses explicit `.js` import specifiers (NodeNext/ESM style); Vite's
// resolver handles those against the `.ts` files transparently.
export default defineConfig({
  test: {
    environment: "node",
    include: ["test/**/*.test.ts"],
  },
});
