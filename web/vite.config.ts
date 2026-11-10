import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Development proxies API calls to a real Vesper server (VESPER_API, default
// http://127.0.0.1:8080). There is no mock data in the application bundle.
const api = process.env.VESPER_API ?? "http://127.0.0.1:8080";

export default defineConfig({
  plugins: [react()],
  server: { proxy: { "/api": { target: api, changeOrigin: false } } },
  build: { outDir: "dist", sourcemap: true },
  test: {
    environment: "jsdom",
    setupFiles: ["./test/setup.ts"],
    include: ["test/**/*.test.{ts,tsx}"],
  },
});
