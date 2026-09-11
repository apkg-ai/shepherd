/// <reference types="vitest/config" />
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": "http://127.0.0.1:7437",
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["src/test-setup.ts"],
    // Vitest owns src/**/*.test.* only — Playwright specs live in e2e/ and
    // would otherwise match vitest's default *.spec.* include.
    include: ["src/**/*.test.{ts,tsx}"],
    coverage: {
      provider: "v8",
      reporter: [["text"], ["lcovonly", { file: "ui-unit.lcov" }]],
      // Explicit include so untested src files count against the gate instead
      // of silently missing from the report (and CSS stays out of the lcov).
      include: ["src/**/*.{ts,tsx}"],
      exclude: [
        // Spec-derived code is gated by ui-generated-drift, not coverage —
        // mirrors core ignoring src/generated/ in llvm-cov.
        "src/api/generated/**",
        // Entry point is exercised by scripts/smoke.sh, mirroring main.rs.
        "src/main.tsx",
        "src/test/**",
        "src/test-setup.ts",
      ],
    },
  },
});
