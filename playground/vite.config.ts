import react from "@vitejs/plugin-react";
import type { Plugin } from "vite";
import { defineConfig } from "vitest/config";
import { createPlaygroundApiMiddleware } from "./server/index";

function playgroundApiPlugin(): Plugin {
  return {
    name: "quaid-playground-api",
    configureServer(server) {
      server.middlewares.use("/api", createPlaygroundApiMiddleware());
    }
  };
}

export default defineConfig({
  plugins: [react(), playgroundApiPlugin()],
  server: {
    host: "127.0.0.1",
    port: 5174,
    strictPort: false
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    exclude: ["node_modules/**", "dist/**", "tests/e2e/**"]
  }
});
