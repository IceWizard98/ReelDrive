import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri expects a fixed port and does not want vite to obscure rust errors.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.js"],
    include: ["src/**/*.test.js"],
  },
  // Svelte 5 ships separate server and browser entry points, and the server one
  // renders to a string instead of to the DOM. Under vitest the default
  // resolution picks the server build, so components mount and nothing appears.
  resolve: process.env.VITEST ? { conditions: ["browser"] } : undefined,
});
