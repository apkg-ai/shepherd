/// <reference types="vitest/config" />
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  // Sourcemaps feed the e2e coverage collector (monocart).
  build: { sourcemap: true },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:7437",
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["src/test-setup.ts"],
    // Keep Playwright specs (e2e/) out of vitest's default *.spec.* include.
    include: ["src/**/*.test.{ts,tsx}"],
    coverage: {
      provider: "v8",
      reporter: [["text"], ["lcovonly", { file: "ui-unit.lcov" }]],
      // Explicit include: untested src files count against the gate; CSS stays out.
      include: ["src/**/*.{ts,tsx}"],
      exclude: [
        "src/api/generated/**",
        // Entry point is exercised by scripts/smoke.sh, mirroring main.rs.
        "src/main.tsx",
        "src/test/**",
        "src/test-setup.ts",
      ],
    },
  },
});
