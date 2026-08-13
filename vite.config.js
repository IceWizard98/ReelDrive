import { spawnSync } from "node:child_process";
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { gitVersion } from "./scripts/git-version.js";

// The footer's number, in order of authority: what the release workflow put in
// the environment, then what the checkout itself says, then nothing — which the
// app renders as "dev". The middle step is for whoever clones this repository
// and builds it: their copy names the commit it was cut from instead of
// pretending to be the same anonymous build as everyone else's.
//
// Read straight off `process.env`, before Vite has loaded any `.env` file. That
// is the same precedence Vite itself uses — `process.env` beats `.env.local` —
// so pinning a version there does not work; pass it on the command line.
//
// Skipped under vitest, where the footer's own tests supply the variable and an
// ambient one would decide what they see.
if (!process.env.VITEST && !process.env.VITE_REELDRIVE_VERSION) {
  process.env.VITE_REELDRIVE_VERSION = gitVersion((cmd, args) =>
    spawnSync(cmd, args, { encoding: "utf8" }),
  );
}

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
