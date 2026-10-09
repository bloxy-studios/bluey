import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { fileURLToPath, URL } from "node:url";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./vitest.setup.ts"],
    exclude: ["node_modules", "dist", "src-tauri", "sidecars"],
    css: false,
    // `include` lives on each project: a root one would be merged into both.
    projects: [
      {
        extends: true,
        test: {
          name: "unit",
          include: ["src/**/*.{test,spec}.{ts,tsx}", "tests/**/*.{test,spec}.{ts,tsx}"],
          exclude: ["tests/ui/**"],
        },
      },
      {
        // Rendered React trees with user-event typing take 5+ s per test on a
        // slower developer Mac; the 5 s default made these flaky (TEST-022).
        extends: true,
        test: { name: "ui", include: ["tests/ui/**/*.{test,spec}.{ts,tsx}"], testTimeout: 15_000 },
      },
    ],
    coverage: {
      provider: "v8",
      reporter: ["text", "html"],
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/**/*.d.ts", "src/main.tsx"],
    },
  },
});
