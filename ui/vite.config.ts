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
    coverage: {
      provider: "v8",
      reporter: [["text"], ["lcovonly", { file: "ui-unit.lcov" }]],
    },
  },
});
